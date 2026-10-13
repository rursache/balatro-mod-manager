import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  activateFocusedElement,
  findNearestFocusTarget,
  getFocusableElements,
  handleControllerButtons,
  moveControllerFocus,
  startControllerNavigation,
} from "./controllerNavigation";

function rect(
  element: HTMLElement,
  left: number,
  top: number,
  width = 40,
  height = 30,
) {
  element.getBoundingClientRect = vi.fn(
    () =>
      ({
        left,
        top,
        right: left + width,
        bottom: top + height,
        width,
        height,
        x: left,
        y: top,
        toJSON: () => {},
      }) as DOMRect,
  );
}

describe("controllerNavigation", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("filters disabled and hidden focusable elements", () => {
    const enabled = document.createElement("button");
    const disabled = document.createElement("button");
    const hidden = document.createElement("button");

    disabled.disabled = true;
    hidden.hidden = true;

    document.body.append(enabled, disabled, hidden);
    rect(enabled, 0, 0);
    rect(disabled, 50, 0);
    rect(hidden, 100, 0);

    expect(getFocusableElements()).toEqual([enabled]);
  });

  it("finds the nearest focus target in the requested direction", () => {
    const current = document.createElement("button");
    const rightNear = document.createElement("button");
    const rightFar = document.createElement("button");
    const below = document.createElement("button");

    document.body.append(current, rightFar, below, rightNear);
    rect(current, 0, 0);
    rect(rightNear, 70, 5);
    rect(rightFar, 160, 0);
    rect(below, 0, 80);

    expect(
      findNearestFocusTarget(current, "right", [
        current,
        rightFar,
        below,
        rightNear,
      ]),
    ).toBe(rightNear);
  });

  it("activates the controller-focused element", () => {
    const button = document.createElement("button");
    const onClick = vi.fn();

    button.addEventListener("click", onClick);
    button.dataset.controllerFocus = "true";
    document.body.append(button);
    rect(button, 0, 0);

    expect(activateFocusedElement()).toBe(true);
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("keeps spatial navigation inside the given modal root", () => {
    const behind = document.createElement("button");
    const dialog = document.createElement("div");
    const confirm = document.createElement("button");
    const cancel = document.createElement("button");
    dialog.append(confirm, cancel);
    document.body.append(behind, dialog);
    rect(behind, 0, 0);
    rect(confirm, 0, 100);
    rect(cancel, 100, 100);

    behind.dataset.controllerFocus = "true";
    expect(moveControllerFocus("down", dialog)).toBe(confirm);
    expect(behind.dataset.controllerFocus).toBeUndefined();
    expect(moveControllerFocus("up", dialog)).toBeNull();
    expect(moveControllerFocus("right", dialog)).toBe(cancel);
    expect(moveControllerFocus("right", dialog)).toBeNull();
    expect(document.activeElement).toBe(cancel);
  });

  it("calls cancel for B and Back button presses", () => {
    const onCancel = vi.fn();
    const onConfirm = vi.fn();

    handleControllerButtons(new Set(), new Set([1]), {
      onCancel,
      onConfirm,
    });
    handleControllerButtons(new Set(), new Set([8]), {
      onCancel,
      onConfirm,
    });

    expect(onCancel).toHaveBeenCalledTimes(2);
    expect(onConfirm).not.toHaveBeenCalled();
  });

  describe("polling loop", () => {
    let gamepads: (Gamepad | null)[];
    let rafMock: ReturnType<typeof vi.fn>;
    let cancelRafMock: ReturnType<typeof vi.fn>;

    function pad(
      index: number,
      pressed: number[] = [],
      axes: number[] = [0, 0],
    ): Gamepad {
      const buttons = Array.from({ length: 16 }, (_, i) => ({
        pressed: pressed.includes(i),
      }));
      return { index, buttons, axes } as unknown as Gamepad;
    }

    function focusTargets() {
      const first = document.createElement("button");
      const second = document.createElement("button");
      document.body.append(first, second);
      rect(first, 0, 0);
      rect(second, 0, 100);
      return { first, second };
    }

    function gamepadEvent(type: string, index: number) {
      const event = new Event(type);
      Object.defineProperty(event, "gamepad", {
        value: gamepads[index] ?? pad(index),
      });
      window.dispatchEvent(event);
    }

    function runFrame() {
      const tick = rafMock.mock.calls.at(-1)?.[0] as FrameRequestCallback;
      tick(performance.now());
    }

    beforeEach(() => {
      gamepads = [null, null];
      rafMock = vi.fn(() => 1);
      cancelRafMock = vi.fn();
      vi.stubGlobal("requestAnimationFrame", rafMock);
      vi.stubGlobal("cancelAnimationFrame", cancelRafMock);
      Object.defineProperty(navigator, "getGamepads", {
        configurable: true,
        value: () => gamepads,
      });
    });

    afterEach(() => {
      vi.unstubAllGlobals();
      Reflect.deleteProperty(navigator, "getGamepads");
    });

    it("only polls while a gamepad is connected", () => {
      const stop = startControllerNavigation({ onCancel: vi.fn() });
      expect(rafMock).not.toHaveBeenCalled();

      gamepads[0] = pad(0);
      gamepadEvent("gamepadconnected", 0);
      expect(rafMock).toHaveBeenCalledTimes(1);

      gamepads[0] = null;
      gamepadEvent("gamepaddisconnected", 0);
      expect(cancelRafMock).toHaveBeenCalled();

      rafMock.mockClear();
      stop();
      gamepadEvent("gamepadconnected", 1);
      expect(rafMock).not.toHaveBeenCalled();
    });

    it("starts right away when a gamepad is already connected", () => {
      gamepads[1] = pad(1);
      const stop = startControllerNavigation({ onCancel: vi.fn() });
      expect(rafMock).toHaveBeenCalledTimes(1);
      stop();
    });

    it("ignores the button press that woke the gamepad", () => {
      const onCancel = vi.fn();
      const stop = startControllerNavigation({ onCancel });

      gamepads[0] = pad(0, [1]);
      gamepadEvent("gamepadconnected", 0);
      runFrame();
      expect(onCancel).not.toHaveBeenCalled();

      gamepads[0] = pad(0);
      runFrame();
      gamepads[0] = pad(0, [1]);
      runFrame();
      expect(onCancel).toHaveBeenCalledTimes(1);
      stop();
    });

    it("ignores the d-pad press that woke the gamepad", () => {
      const { first } = focusTargets();
      const stop = startControllerNavigation({ onCancel: vi.fn() });

      gamepads[0] = pad(0, [13]);
      gamepadEvent("gamepadconnected", 0);
      runFrame();
      runFrame();
      expect(document.activeElement).not.toBe(first);

      gamepads[0] = pad(0);
      runFrame();
      gamepads[0] = pad(0, [13]);
      runFrame();
      expect(document.activeElement).toBe(first);
      stop();
    });

    it("ignores the stick push that woke the gamepad until it changes", () => {
      const { first } = focusTargets();
      const stop = startControllerNavigation({ onCancel: vi.fn() });

      gamepads[0] = pad(0, [], [0, 1]);
      gamepadEvent("gamepadconnected", 0);
      runFrame();
      expect(document.activeElement).not.toBe(first);

      gamepads[0] = pad(0, [], [1, 0]);
      runFrame();
      expect(document.activeElement).toBe(first);
      stop();
    });

    it("seeds the direction of a pad that is already connected", () => {
      const { first } = focusTargets();
      gamepads[0] = pad(0, [12]);
      const stop = startControllerNavigation({ onCancel: vi.fn() });
      runFrame();
      expect(document.activeElement).not.toBe(first);
      stop();
    });
  });
});
