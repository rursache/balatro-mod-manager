import { get } from "svelte/store";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InstalledMod, Mod } from "./modStore";
import {
  loadingStates2,
  modEnabledStore,
  showWarningPopup,
  updateAvailableStore,
} from "./modStore";

const invokeMock = vi.hoisted(() => vi.fn());
const rowsMock = vi.hoisted(() => ({ rows: [] as InstalledMod[] }));
const addMessageMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("$lib/stores", () => ({
  addMessage: (...args: unknown[]) => addMessageMock(...args),
}));

vi.mock("./modCache", () => ({
  fetchCachedMods: vi.fn(async () => rowsMock.rows),
  forceRefreshCache: vi.fn(async () => rowsMock.rows),
}));

import { confirmReinstall, reinstallMod } from "./reinstall";

const base: Mod = {
  title: "Some Mod",
  description: "",
  image: "",
  categories: [],
  colors: { color1: "#000", color2: "#111" },
  requires_steamodded: false,
  requires_talisman: false,
  publisher: "pub",
  repo: "",
  downloadURL: "https://example.com/some.zip",
  installed: true,
  last_updated: 0,
  _dirName: "pub@SomeMod",
  source: "index",
  version: "1.2.0",
};

const jokerDisplay: Mod = {
  ...base,
  title: "JokerDisplay",
  _dirName: "ts:nh6574-JokerDisplay",
  source: "thunderstore",
  thunderstore: {
    namespace: "nh6574",
    name: "JokerDisplay",
    full_name: "nh6574-JokerDisplay",
  } as Mod["thunderstore"],
};

const steamodded: Mod = {
  ...base,
  title: "Steamodded",
  _dirName: "Steamodded@smods",
  source: "merged",
};

function commandsCalled(): string[] {
  return invokeMock.mock.calls.map(([c]) => c as string);
}

function argsOf(command: string): Record<string, unknown> | null {
  return invokeMock.mock.calls.find(([c]) => c === command)?.[1] ?? null;
}

describe("reinstallMod", () => {
  let enabled: boolean;

  beforeEach(() => {
    vi.clearAllMocks();
    enabled = true;
    loadingStates2.set({});
    modEnabledStore.set({});
    updateAvailableStore.set({});
    invokeMock.mockImplementation(
      async (command: string, args: Record<string, unknown>) => {
        switch (command) {
          case "is_mod_enabled":
            return enabled;
          case "toggle_mod_enabled":
            enabled = args.enabled as boolean;
            return null;
          case "install_catalog_mod":
            // A reinstall replaces the folder, dropping its .lovelyignore
            enabled = true;
            return {
              plan: {},
              installed: [
                {
                  name: "JokerDisplay",
                  path: "/Mods/nh6574-JokerDisplay",
                  version: args.version,
                  source: "thunderstore",
                  source_id: "nh6574-JokerDisplay",
                },
              ],
              warnings: [],
            };
          case "install_mod":
            enabled = true;
            return "/Mods/SomeMod";
          case "install_steamodded_version":
            enabled = true;
            return `/Mods/smods-${args.version}`;
          case "verify_path_exists":
            return true;
          default:
            return null;
        }
      },
    );
  });

  it("reinstalls the installed Thunderstore version through the catalog", async () => {
    rowsMock.rows = [
      {
        name: "JokerDisplay",
        path: "/Mods/nh6574-JokerDisplay",
        source: "thunderstore",
        source_id: "nh6574-JokerDisplay",
        current_version: "2.0.5",
      },
    ];
    const result = await reinstallMod(jokerDisplay, { confirm: false });
    expect(result).toEqual({ name: "JokerDisplay", version: "2.0.5" });
    expect(argsOf("install_catalog_mod")).toEqual({
      id: "ts:nh6574-JokerDisplay",
      version: "2.0.5",
      source: "thunderstore",
      installSteamodded: false,
    });
    expect(get(loadingStates2)["JokerDisplay"]).toBe(false);
  });

  it("keeps a disabled mod disabled", async () => {
    enabled = false;
    rowsMock.rows = [
      {
        name: "JokerDisplay",
        path: "/Mods/nh6574-JokerDisplay",
        source: "thunderstore",
        source_id: "nh6574-JokerDisplay",
        current_version: "2.0.5",
      },
    ];
    await reinstallMod(jokerDisplay, { confirm: false });
    expect(argsOf("toggle_mod_enabled")).toEqual({
      modName: "JokerDisplay",
      enabled: false,
    });
    expect(enabled).toBe(false);
    expect(get(modEnabledStore)["JokerDisplay"]).toBe(false);
  });

  it("does not touch the enabled state of an enabled mod", async () => {
    rowsMock.rows = [
      { name: "Some Mod", path: "/Mods/SomeMod", source: "index" },
    ];
    await reinstallMod(base, { confirm: false });
    expect(commandsCalled()).not.toContain("toggle_mod_enabled");
  });

  it("reinstalls index mods from the download URL into the same folder", async () => {
    rowsMock.rows = [
      {
        name: "Some Mod",
        path: "/Mods/SomeModFolder",
        source: "index",
        dependencies: ["Steamodded"],
        current_version: "1.1.0",
      },
    ];
    updateAvailableStore.set({ "Some Mod": true });
    const result = await reinstallMod(base, { confirm: false });
    expect(argsOf("install_mod")).toEqual({
      url: "https://example.com/some.zip",
      folderName: "SomeModFolder",
    });
    expect(argsOf("add_installed_mod")).toEqual({
      name: "Some Mod",
      path: "/Mods/SomeMod",
      dependencies: ["Steamodded"],
      currentVersion: "1.2.0",
    });
    expect(result?.version).toBe("1.2.0");
    expect(get(updateAvailableStore)["Some Mod"]).toBe(false);
  });

  it("reinstalls the picked Steamodded tag and records it", async () => {
    rowsMock.rows = [
      {
        name: "Steamodded",
        path: "/Mods/smods-26.1002.0",
        source: "index",
        current_version: "26.1002.0",
      },
    ];
    await reinstallMod(steamodded, { confirm: false, version: "26.829.0" });
    expect(argsOf("install_steamodded_version")).toEqual({
      version: "26.829.0",
    });
    expect(argsOf("add_installed_mod")).toMatchObject({
      name: "Steamodded",
      path: "/Mods/smods-26.829.0",
      currentVersion: "26.829.0",
    });
    expect(commandsCalled()).not.toContain("install_catalog_mod");
  });

  it("reports failures and clears the loading state", async () => {
    rowsMock.rows = [
      { name: "Some Mod", path: "/Mods/SomeMod", source: "index" },
    ];
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "install_mod") throw new Error("network down");
      return command === "is_mod_enabled" ? true : null;
    });
    expect(await reinstallMod(base, { confirm: false })).toBeNull();
    expect(addMessageMock).toHaveBeenCalledWith(
      "Failed to reinstall Some Mod: network down",
      "error",
    );
    expect(commandsCalled()).not.toContain("add_installed_mod");
    expect(get(loadingStates2)["Some Mod"]).toBe(false);
  });

  it("skips local installs", async () => {
    rowsMock.rows = [{ name: "Local", path: "/Mods/Local", source: "index" }];
    const local: Mod = {
      ...base,
      title: "Local",
      _dirName: undefined,
      _installedPath: "/Mods/Local",
    };
    expect(await reinstallMod(local, { confirm: false })).toBeNull();
    expect(commandsCalled()).toEqual([]);
  });

  it("asks first and does nothing when cancelled", async () => {
    rowsMock.rows = [
      { name: "Some Mod", path: "/Mods/SomeMod", source: "index" },
    ];
    const pending = reinstallMod(base);
    await vi.waitFor(() => expect(get(showWarningPopup).visible).toBe(true));
    expect(get(showWarningPopup).message).toContain("Reinstall Some Mod 1.2.0");
    get(showWarningPopup).onCancel();
    expect(await pending).toBeNull();
    expect(commandsCalled()).not.toContain("install_mod");
  });

  it("confirmReinstall resolves true on confirm", async () => {
    const pending = confirmReinstall(base, "1.2.0");
    get(showWarningPopup).onConfirm();
    expect(await pending).toBe(true);
  });
});
