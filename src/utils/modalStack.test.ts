import { get } from "svelte/store";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  getInitialFocusTarget,
  getTopModal,
  getTrappedTabTarget,
  isModalOpen,
  modal,
  modalOpen,
  resetModalStack,
} from "./modalStack";

function popup(...labels: string[]) {
  const root = document.createElement("div");
  for (const label of labels) {
    const button = document.createElement("button");
    button.textContent = label;
    root.append(button);
  }
  document.body.append(root);
  return root;
}

function pressKey(key: string, init: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", {
    key,
    bubbles: true,
    cancelable: true,
    ...init,
  });
  (document.activeElement ?? document.body).dispatchEvent(event);
  return event;
}

describe("modalStack", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    resetModalStack();
  });

  afterEach(() => {
    resetModalStack();
  });

  it("closes only the topmost popup on Escape", () => {
    const onLower = vi.fn();
    const onUpper = vi.fn();
    const lower = modal(popup("Lower"), { onEscape: onLower });
    const upper = modal(popup("Upper"), { onEscape: onUpper });

    pressKey("Escape");
    expect(onUpper).toHaveBeenCalledOnce();
    expect(onLower).not.toHaveBeenCalled();

    upper.destroy();
    pressKey("Escape");
    expect(onLower).toHaveBeenCalledOnce();
    expect(onUpper).toHaveBeenCalledOnce();
    lower.destroy();
  });

  it("consumes Escape so window listeners behind the popup ignore it", () => {
    const onEscape = vi.fn();
    const viewBack = vi.fn();
    const onWindowKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      viewBack();
    };
    const handle = modal(popup("Close"), { onEscape });
    window.addEventListener("keydown", onWindowKey);

    const event = pressKey("Escape");
    expect(event.defaultPrevented).toBe(true);
    expect(onEscape).toHaveBeenCalledOnce();
    expect(viewBack).not.toHaveBeenCalled();

    window.removeEventListener("keydown", onWindowKey);
    handle.destroy();
  });

  it("handles the synthetic Escape the controller dispatches on window", () => {
    const onEscape = vi.fn();
    const handle = modal(popup("Close"), { onEscape });
    window.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", cancelable: true }),
    );
    expect(onEscape).toHaveBeenCalledOnce();
    handle.destroy();
  });

  it("leaves Escape alone when an inner widget already claimed it", () => {
    const onEscape = vi.fn();
    const root = popup("Dropdown");
    const handle = modal(root, { onEscape });
    root
      .querySelector("button")!
      .addEventListener("keydown", (event) => event.preventDefault());
    root.querySelector("button")!.focus();

    pressKey("Escape");
    expect(onEscape).not.toHaveBeenCalled();
    handle.destroy();
  });

  it("hands Escape to the popup below once the top one starts closing", () => {
    const onLower = vi.fn();
    const onUpper = vi.fn();
    const lower = modal(popup("Lower"), { onEscape: onLower });
    const upperRoot = popup("Upper");
    const upper = modal(upperRoot, { onEscape: onUpper });

    upperRoot.setAttribute("inert", "");
    expect(isModalOpen()).toBe(true);
    pressKey("Escape");
    expect(onUpper).not.toHaveBeenCalled();
    expect(onLower).toHaveBeenCalledOnce();

    upper.destroy();
    lower.destroy();
  });

  it("tracks open state and releases the popup when its outro starts", () => {
    const root = popup("Close");
    const handle = modal(root, { onEscape: vi.fn() });
    expect(isModalOpen()).toBe(true);
    expect(get(modalOpen)).toBe(true);
    expect(getTopModal()).toBe(root);

    root.dispatchEvent(new CustomEvent("outrostart"));
    expect(isModalOpen()).toBe(false);
    expect(get(modalOpen)).toBe(false);
    expect(getTopModal()).toBeNull();

    root.dispatchEvent(new CustomEvent("introstart"));
    expect(getTopModal()).toBe(root);
    handle.destroy();
    expect(isModalOpen()).toBe(false);
  });

  it("moves focus into the popup and restores it on close", () => {
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();

    const root = popup("Confirm", "Cancel");
    const cancel = root.querySelectorAll("button")[1];
    cancel.dataset.autofocus = "";
    const handle = modal(root, { onEscape: vi.fn() });
    expect(document.activeElement).toBe(cancel);

    handle.destroy();
    expect(document.activeElement).toBe(opener);
  });

  it("traps Tab inside the topmost popup", () => {
    const root = popup("First", "Second");
    const [first, second] = root.querySelectorAll("button");
    const handle = modal(root, { onEscape: vi.fn() });
    expect(document.activeElement).toBe(first);

    pressKey("Tab");
    expect(document.activeElement).toBe(second);
    pressKey("Tab");
    expect(document.activeElement).toBe(first);
    pressKey("Tab", { shiftKey: true });
    expect(document.activeElement).toBe(second);
    handle.destroy();
  });

  describe("focus trap selection", () => {
    const a = document.createElement("button");
    const b = document.createElement("button");
    const c = document.createElement("button");
    const outside = document.createElement("button");

    it("wraps forward and backward", () => {
      expect(getTrappedTabTarget([a, b, c], a, false)).toBe(b);
      expect(getTrappedTabTarget([a, b, c], c, false)).toBe(a);
      expect(getTrappedTabTarget([a, b, c], a, true)).toBe(c);
      expect(getTrappedTabTarget([a, b, c], b, true)).toBe(a);
    });

    it("pulls focus back in from outside the popup", () => {
      expect(getTrappedTabTarget([a, b, c], outside, false)).toBe(a);
      expect(getTrappedTabTarget([a, b, c], outside, true)).toBe(c);
      expect(getTrappedTabTarget([a, b, c], null, false)).toBe(a);
    });

    it("returns nothing when the popup has no focusable elements", () => {
      expect(getTrappedTabTarget([], a, false)).toBeNull();
    });

    it("prefers the autofocus element and skips disabled ones", () => {
      const root = popup("Disabled", "Confirm", "Cancel");
      const [disabled, confirm, cancel] = root.querySelectorAll("button");
      disabled.disabled = true;
      expect(getInitialFocusTarget(root)).toBe(confirm);

      cancel.dataset.autofocus = "";
      expect(getInitialFocusTarget(root)).toBe(cancel);
    });
  });
});
