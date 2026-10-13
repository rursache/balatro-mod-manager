import { fireEvent, render } from "@testing-library/svelte";
import { tick } from "svelte";
import { get } from "svelte/store";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ModView from "./ModView.svelte";
import WarningPopup from "../WarningPopup.svelte";
import {
  currentModView,
  installationStatus,
  showWarningPopup,
} from "../../stores/modStore";
import type { Mod } from "../../stores/modStore";
import { resetModalStack } from "../../utils/modalStack";
import { forceRefreshCache } from "../../stores/modCache";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("$lib/stores", () => ({
  addMessage: vi.fn(),
}));

vi.mock("$lib/platform", () => ({
  isLinuxPlatform: vi.fn().mockResolvedValue(false),
}));

vi.mock("$lib/opener", () => ({
  openExternal: vi.fn(),
}));

if (!("animate" in Element.prototype)) {
  Object.defineProperty(Element.prototype, "animate", {
    value: () => ({
      cancel: () => {},
      play: () => {},
      pause: () => {},
      finished: Promise.resolve(),
    }),
    writable: true,
    configurable: true,
  });
}

const mod: Mod = {
  title: "Test Mod",
  description: "desc",
  image: "img",
  categories: [],
  colors: { color1: "#000", color2: "#111" },
  requires_steamodded: false,
  requires_talisman: false,
  publisher: "pub",
  repo: "https://github.com/pub/test-mod",
  downloadURL: "https://example.com/test.zip",
  installed: false,
  last_updated: 0,
};

function commandResult(command: string) {
  switch (command) {
    case "get_installed_mods_from_db":
    case "get_dependents":
    case "get_steamodded_versions":
    case "get_talisman_versions":
      return [];
    case "get_mod_requirements":
      return [false, false];
    case "get_description_cached_or_remote":
      return "";
    default:
      return null;
  }
}

function pressEscape() {
  window.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", cancelable: true }),
  );
}

describe("ModView Escape handling", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeMock.mockImplementation(async (command: string) =>
      commandResult(command),
    );
    resetModalStack();
    showWarningPopup.set({
      visible: false,
      message: "",
      onConfirm: () => {},
      onCancel: () => {},
    });
    currentModView.set(mod);
  });

  afterEach(() => {
    resetModalStack();
    currentModView.set(null);
  });

  it("goes back on Escape when no popup is open", async () => {
    render(ModView, { props: { mod } });
    await tick();

    pressEscape();
    expect(get(currentModView)).toBeNull();
  });

  it("ignores Escape while a popup is open and lets the popup close", async () => {
    const onCancel = vi.fn();
    render(ModView, { props: { mod } });
    render(WarningPopup, {
      props: { visible: true, message: "Sure?", onConfirm: vi.fn(), onCancel },
    });
    await tick();

    pressEscape();
    expect(onCancel).toHaveBeenCalledOnce();
    expect(get(currentModView)).toBe(mod);
  });

  it("ignores Escape while a managed popup store is visible", async () => {
    render(ModView, { props: { mod } });
    showWarningPopup.set({
      visible: true,
      message: "Sure?",
      onConfirm: () => {},
      onCancel: () => {},
    });
    await tick();

    pressEscape();
    expect(get(currentModView)).toBe(mod);
  });

  it("does not go back on Backspace inside a text field", async () => {
    render(ModView, { props: { mod } });
    await tick();
    const input = document.createElement("input");
    document.body.append(input);

    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Backspace", bubbles: true }),
    );
    expect(get(currentModView)).toBe(mod);
    input.remove();
  });
});

describe("ModView dependency plan", () => {
  const tsMod: Mod = {
    ...mod,
    title: "JokerDisplay",
    _dirName: "ts:nh6574-JokerDisplay",
    source: "thunderstore",
    thunderstore: {
      namespace: "nh6574",
      name: "JokerDisplay",
      full_name: "nh6574-JokerDisplay",
      package_url: "",
    } as Mod["thunderstore"],
  };
  const plan = {
    mod_id: "ts:nh6574-JokerDisplay",
    mod_title: "JokerDisplay",
    source: "thunderstore",
    version: "1.0.0",
    steps: [
      {
        source: "thunderstore",
        catalog_id: "ts:nh6574-JokerDisplay",
        title: "JokerDisplay",
        full_name: "nh6574-JokerDisplay",
        version: "1.0.0",
        is_root: true,
        deprecated: false,
      },
    ],
    satisfied: [],
    lovely: null,
    steamodded: null,
    talisman: null,
    missing: [],
    warnings: [],
  };

  beforeEach(() => {
    vi.clearAllMocks();
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "plan_catalog_install") return plan;
      if (command === "install_catalog_mod") {
        return { plan, installed: [], warnings: [] };
      }
      return commandResult(command);
    });
    resetModalStack();
    currentModView.set(tsMod);
  });

  afterEach(() => {
    resetModalStack();
    currentModView.set(null);
  });

  it("plans again after an install", async () => {
    const { getByRole } = render(ModView, { props: { mod: tsMod } });
    const planCalls = () =>
      invokeMock.mock.calls.filter(([c]) => c === "plan_catalog_install")
        .length;
    await vi.waitFor(() => expect(planCalls()).toBe(1));

    getByRole("button", { name: /download/i }).click();
    await vi.waitFor(() =>
      expect(
        invokeMock.mock.calls.some(([c]) => c === "install_catalog_mod"),
      ).toBe(true),
    );
    // One plan for the install itself and a fresh one for the view
    await vi.waitFor(() => expect(planCalls()).toBe(3));
  });
});

describe("ModView modal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeMock.mockImplementation(async (command: string) =>
      commandResult(command),
    );
    resetModalStack();
    installationStatus.set({});
    showWarningPopup.set({
      visible: false,
      message: "",
      onConfirm: () => {},
      onCancel: () => {},
    });
    currentModView.set(mod);
  });

  afterEach(() => {
    resetModalStack();
    currentModView.set(null);
  });

  it("renders as a dialog that traps focus on the close button", async () => {
    const { getByRole } = render(ModView, { props: { mod } });
    await tick();
    const dialog = getByRole("dialog");
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(document.activeElement).toBe(getByRole("button", { name: "Close" }));
  });

  it("closes from the close button", async () => {
    const { getByRole } = render(ModView, { props: { mod } });
    await tick();
    await fireEvent.click(getByRole("button", { name: "Close" }));
    expect(get(currentModView)).toBeNull();
  });

  it("closes on a backdrop click but not on a click inside the panel", async () => {
    const { container, getByRole } = render(ModView, { props: { mod } });
    await tick();
    const dialog = getByRole("dialog");
    await fireEvent.mouseDown(dialog);
    await fireEvent.click(dialog);
    expect(get(currentModView)).toBe(mod);

    // A drag that starts inside and ends on the backdrop keeps the modal open
    const backdrop = container.querySelector(
      ".mod-view-backdrop",
    ) as HTMLElement;
    await fireEvent.mouseDown(dialog);
    await fireEvent.click(backdrop);
    expect(get(currentModView)).toBe(mod);

    await fireEvent.mouseDown(backdrop);
    await fireEvent.click(backdrop);
    expect(get(currentModView)).toBeNull();
  });

  it("closes the confirmation above it before itself on Escape", async () => {
    render(ModView, { props: { mod } });
    const onCancel = vi.fn();
    showWarningPopup.set({
      visible: true,
      message: "Reinstall?",
      onConfirm: vi.fn(),
      onCancel,
    });
    render(WarningPopup, {
      props: {
        visible: true,
        message: "Reinstall?",
        onConfirm: vi.fn(),
        onCancel,
      },
    });
    await tick();

    pressEscape();
    expect(onCancel).toHaveBeenCalledOnce();
    expect(get(currentModView)).toBe(mod);
  });

  it("shows Steamodded's source as information without a switch", async () => {
    const smods: Mod = {
      ...mod,
      title: "Steamodded",
      _dirName: "Steamodded@smods",
      source: "merged",
      thunderstore: {
        namespace: "Steamodded",
        name: "Steamodded",
        full_name: "Steamodded-Steamodded",
        package_url:
          "https://thunderstore.io/c/balatro/p/Steamodded/Steamodded/",
      } as Mod["thunderstore"],
    };
    currentModView.set(smods);
    const { queryByTestId, getByTestId, queryByRole } = render(ModView, {
      props: { mod: smods },
    });
    await tick();
    expect(queryByTestId("source-picker")).toBeNull();
    expect(getByTestId("source-info").textContent).toContain("GitHub releases");
    expect(queryByRole("button", { name: /switch/i })).toBeNull();
  });

  it("shows a segmented source picker for other merged mods", async () => {
    const merged: Mod = {
      ...mod,
      _dirName: "pub@TestMod",
      source: "merged",
      thunderstore: {
        namespace: "pub",
        name: "TestMod",
        full_name: "pub-TestMod",
        package_url: "",
      } as Mod["thunderstore"],
    };
    currentModView.set(merged);
    const { getByTestId } = render(ModView, { props: { mod: merged } });
    await tick();
    const picker = getByTestId("source-picker");
    expect(picker.querySelectorAll("button")).toHaveLength(2);
  });
});

describe("ModView reinstall", () => {
  const jokerDisplay: Mod = {
    ...mod,
    title: "JokerDisplay",
    _dirName: "ts:nh6574-JokerDisplay",
    source: "thunderstore",
    thunderstore: {
      namespace: "nh6574",
      name: "JokerDisplay",
      full_name: "nh6574-JokerDisplay",
      package_url: "",
    } as Mod["thunderstore"],
  };
  const row = {
    name: "JokerDisplay",
    path: "/Mods/nh6574-JokerDisplay",
    source: "thunderstore",
    source_id: "nh6574-JokerDisplay",
    current_version: "2.0.5",
    dependencies: [],
  };

  beforeEach(async () => {
    vi.clearAllMocks();
    resetModalStack();
    invokeMock.mockImplementation(
      async (command: string, args: Record<string, unknown>) => {
        switch (command) {
          case "get_installed_mods_from_db":
            return [row];
          case "is_mod_enabled":
            return true;
          case "plan_catalog_install":
            return {
              steps: [],
              satisfied: [],
              lovely: null,
              steamodded: null,
              talisman: null,
              missing: [],
              warnings: [],
            };
          case "install_catalog_mod":
            return {
              plan: {},
              installed: [{ ...row, version: args.version }],
              warnings: [],
            };
          default:
            return commandResult(command);
        }
      },
    );
    installationStatus.set({ JokerDisplay: true });
    currentModView.set(jokerDisplay);
    await forceRefreshCache();
  });

  afterEach(() => {
    resetModalStack();
    installationStatus.set({});
    currentModView.set(null);
  });

  it("asks, then reinstalls the installed version", async () => {
    const { findByRole } = render(ModView, { props: { mod: jokerDisplay } });
    const button = await findByRole("button", { name: "Reinstall" });
    await fireEvent.click(button);
    await vi.waitFor(() => expect(get(showWarningPopup).visible).toBe(true));
    get(showWarningPopup).onConfirm();
    await vi.waitFor(() =>
      expect(
        invokeMock.mock.calls.find(([c]) => c === "install_catalog_mod")?.[1],
      ).toMatchObject({ version: "2.0.5", source: "thunderstore" }),
    );
  });
});
