import { beforeEach, describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

const invokeMock = vi.hoisted(() => vi.fn());
const addMessageMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));
vi.mock("$lib/stores", () => ({
  addMessage: (...args: unknown[]) => addMessageMock(...args),
}));

import {
  closeInstallPlanPrompt,
  installCatalogEntry,
  installPlanPromptStore,
  planNeedsPrompt,
  type InstallPlan,
  type PlanStep,
} from "./catalogInstall";
import {
  installationStatus,
  loadingStates2,
  modsStore,
  type Mod,
  type ThunderstoreInfo,
} from "./modStore";

const step = (title: string, isRoot = false): PlanStep => ({
  source: "thunderstore",
  catalog_id: `ts:a-${title}`,
  title,
  full_name: `a-${title}`,
  version: "1.0.0",
  min_version: null,
  download_url: "",
  folder_name: `a-${title}`,
  requires: [],
  requires_steamodded: false,
  requires_talisman: false,
  deprecated: false,
  tracked_name: null,
  tracked_source: null,
  installed_version: null,
  is_root: isRoot,
});

const plan = (extra: Partial<InstallPlan> = {}): InstallPlan => ({
  mod_id: "ts:a-Root",
  mod_title: "Root",
  source: "thunderstore",
  version: "1.0.0",
  steps: [step("Root", true)],
  satisfied: [],
  lovely: null,
  steamodded: null,
  talisman: null,
  missing: [],
  warnings: [],
  ...extra,
});

const mod: Mod = {
  title: "Root",
  description: "",
  image: "",
  categories: [],
  colors: { color1: "", color2: "" },
  requires_steamodded: false,
  requires_talisman: false,
  publisher: "a",
  repo: "",
  downloadURL: "ts://a-Root",
  installed: false,
  last_updated: 0,
  _dirName: "ts:a-Root",
  source: "thunderstore",
  thunderstore: { full_name: "a-Root" } as ThunderstoreInfo,
};

function commands() {
  return invokeMock.mock.calls.map(([cmd]) => cmd);
}

describe("planNeedsPrompt", () => {
  it("skips the prompt for a mod without extra work", () => {
    expect(planNeedsPrompt(plan())).toBe(false);
  });

  it("asks when dependencies, missing frameworks or warnings show up", () => {
    expect(
      planNeedsPrompt(plan({ steps: [step("Lib"), step("Root", true)] })),
    ).toBe(true);
    expect(planNeedsPrompt(plan({ missing: ["Talisman"] }))).toBe(true);
    expect(planNeedsPrompt(plan({ warnings: ["old"] }))).toBe(true);
    expect(
      planNeedsPrompt(
        plan({
          lovely: {
            min_version: "0.7.0",
            installed_version: "0.6.0",
            satisfied: false,
          },
        }),
      ),
    ).toBe(true);
  });
});

describe("installCatalogEntry", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    addMessageMock.mockReset();
    installationStatus.set({});
    loadingStates2.set({});
  });

  it("installs straight away when the plan has nothing to confirm", async () => {
    modsStore.set([mod]);
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "plan_catalog_install") return plan();
      if (cmd === "install_catalog_mod") {
        return { plan: plan(), installed: [{ name: "Root" }], warnings: [] };
      }
      if (cmd === "get_installed_mods_from_db") {
        return [
          {
            name: "Root",
            path: "/Mods/a-Root",
            dependencies: [],
            source: "thunderstore",
            source_id: "a-Root",
          },
        ];
      }
      return [];
    });

    await expect(installCatalogEntry(mod)).resolves.toBe(true);
    await vi.waitFor(() =>
      expect(commands()).toContain("get_installed_mods_from_db"),
    );
    // The refresh rebuilds the flags from the DB row, matched by package id
    await vi.waitFor(() => expect(get(installationStatus).Root).toBe(true));
    expect(invokeMock).toHaveBeenCalledWith("install_catalog_mod", {
      id: "ts:a-Root",
      version: undefined,
      source: undefined,
      installSteamodded: false,
    });
    expect(get(installationStatus).Root).toBe(true);
    expect(get(loadingStates2).Root).toBe(false);
  });

  it("waits for the prompt and passes the Steamodded choice", async () => {
    const needsDeps = plan({
      steps: [step("Lib"), step("Root", true)],
      steamodded: {
        min_version: "1.1814.0",
        min_tag: "1.0.0-beta-1814a",
        installed: false,
        installed_version: null,
        satisfied: false,
        legacy_api: false,
      },
      missing: ["Lib", "Steamodded"],
    });
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "plan_catalog_install") return needsDeps;
      if (cmd === "install_catalog_mod") {
        return {
          plan: needsDeps,
          installed: [{ name: "Lib" }, { name: "Root" }],
          warnings: ["heads up"],
        };
      }
      return [];
    });

    const pending = installCatalogEntry(mod, { source: "thunderstore" });
    await vi.waitFor(() => expect(get(installPlanPromptStore).open).toBe(true));
    expect(commands()).not.toContain("install_catalog_mod");
    closeInstallPlanPrompt({ confirmed: true, installSteamodded: true });

    await expect(pending).resolves.toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("install_catalog_mod", {
      id: "ts:a-Root",
      version: undefined,
      source: "thunderstore",
      installSteamodded: true,
    });
    expect(addMessageMock).toHaveBeenCalledWith("heads up", "warning");
  });

  it("does nothing when the prompt is cancelled", async () => {
    invokeMock.mockImplementation(async (cmd: string) =>
      cmd === "plan_catalog_install"
        ? plan({ steps: [step("Lib"), step("Root", true)] })
        : [],
    );

    const pending = installCatalogEntry(mod);
    await vi.waitFor(() => expect(get(installPlanPromptStore).open).toBe(true));
    closeInstallPlanPrompt({ confirmed: false, installSteamodded: false });

    await expect(pending).resolves.toBe(false);
    expect(commands()).not.toContain("install_catalog_mod");
    expect(get(loadingStates2).Root).toBe(false);
  });

  it("reports planner errors", async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === "plan_catalog_install") throw "Unknown mod";
      return [];
    });
    await expect(installCatalogEntry(mod)).resolves.toBe(false);
    expect(addMessageMock).toHaveBeenCalledWith(
      "Installation failed: Unknown mod",
      "error",
    );
  });
});
