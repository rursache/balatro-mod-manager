import { render } from "@testing-library/svelte";
import { tick } from "svelte";
import { get } from "svelte/store";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import ModView from "./ModView.svelte";
import WarningPopup from "../WarningPopup.svelte";
import { currentModView, showWarningPopup } from "../../stores/modStore";
import type { Mod } from "../../stores/modStore";
import { resetModalStack } from "../../utils/modalStack";

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
