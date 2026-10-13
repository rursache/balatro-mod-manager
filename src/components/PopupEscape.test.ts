import { render, screen } from "@testing-library/svelte";
import { tick } from "svelte";
import { get } from "svelte/store";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import WarningPopup from "./WarningPopup.svelte";
import RestoreBackupPopup from "./RestoreBackupPopup.svelte";
import SecurityPopup from "./SecurityPopup.svelte";
import {
  restoreBackupPopupStore,
  securityPopupStore,
} from "../stores/modStore";
import { isModalOpen, resetModalStack } from "../utils/modalStack";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("$lib/stores", () => ({
  addMessage: vi.fn(),
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

function pressEscape() {
  window.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", cancelable: true }),
  );
}

describe("popup Escape handling", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeMock.mockResolvedValue(null);
    resetModalStack();
    restoreBackupPopupStore.set({
      visible: false,
      backupId: "",
      backupName: "",
    });
    securityPopupStore.set({
      visible: false,
      onAcknowledge: () => {},
      onCancel: () => {},
    });
  });

  afterEach(() => {
    resetModalStack();
  });

  it("closes only the topmost popup", async () => {
    const onCancel = vi.fn();
    const onConfirm = vi.fn();
    render(WarningPopup, {
      props: { visible: true, message: "Exit?", onConfirm, onCancel },
    });
    render(RestoreBackupPopup);
    restoreBackupPopupStore.set({
      visible: true,
      backupId: "b1",
      backupName: "Backup",
    });
    await tick();
    expect(screen.getByText("Restore Backup?")).toBeTruthy();

    pressEscape();
    await tick();
    expect(get(restoreBackupPopupStore).visible).toBe(false);
    expect(onCancel).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalledWith(
      "restore_backup",
      expect.anything(),
    );

    pressEscape();
    expect(onCancel).toHaveBeenCalledOnce();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("moves focus to the safe choice when a destructive popup opens", async () => {
    render(RestoreBackupPopup);
    restoreBackupPopupStore.set({
      visible: true,
      backupId: "b1",
      backupName: "Backup",
    });
    await tick();
    expect(document.activeElement?.textContent?.trim()).toBe("Cancel");
  });

  it("dismisses the security notice without quitting or acknowledging", async () => {
    const onAcknowledge = vi.fn();
    const onCancel = vi.fn();
    render(SecurityPopup);
    securityPopupStore.set({ visible: true, onAcknowledge, onCancel });
    await tick();
    expect(isModalOpen()).toBe(true);

    pressEscape();
    await tick();
    expect(onCancel).toHaveBeenCalledOnce();
    expect(onAcknowledge).not.toHaveBeenCalled();
    expect(invokeMock).not.toHaveBeenCalled();
    expect(get(securityPopupStore).visible).toBe(false);
  });
});
