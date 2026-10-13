import { writable, get, derived } from "svelte/store";
import {
  lovelyPopupStore,
  uninstallDialogStore,
  showWarningPopup,
  requiresPopupStore,
  securityPopupStore,
  updatePopupStore,
  reportIssueStore,
  createBackupPopupStore,
  restoreBackupPopupStore,
  deleteBackupPopupStore,
  type LovelyPopupState,
  type UninstallDialogState,
  type WarningPopupState,
  type RequiresPopupState,
  type SecurityPopupState,
  type UpdatePopupState,
  type ReportIssueState,
  type CreateBackupPopupState,
  type RestoreBackupPopupState,
  type DeleteBackupPopupState,
} from "./modStore";
import {
  collectionImportStore,
  collectionPickerStore,
  depPromptStore,
  type DepPromptState,
} from "./collections";
import { isModalOpen, modalOpen } from "../utils/modalStack";

/**
 * Animation duration for popup transitions (in ms).
 * Should match the CSS transition durations in popup components.
 */
const ANIMATION_DURATION = 120;

/**
 * Maximum lifetime of the transitioning flag. The longest legitimate chain
 * is ANIMATION_DURATION + a microtask, so 600ms is generous. If we exceed
 * it something dropped the clear-call (Tauri webview hiccup, navigation
 * race) and we must release the click-blocker overlay so the UI stays
 * interactive.
 */
const TRANSITION_WATCHDOG_MS = 600;

/**
 * Popup types that can be managed by the popup manager.
 */
export type PopupType =
  | "lovely"
  | "uninstall"
  | "warning"
  | "collectionImport"
  | "collectionPicker"
  | "depPrompt"
  | "requires"
  | "security"
  | "update"
  | "reportIssue"
  | "createBackup"
  | "restoreBackup"
  | "deleteBackup";

/**
 * Represents the state of a popup that has been pushed to the stack.
 */
export interface PopupState {
  type: PopupType;
  data: unknown;
}

/**
 * Stack of popups that were open before the current one.
 * When the current popup closes, we restore the previous one.
 */
const popupStack = writable<PopupState[]>([]);

/**
 * Flag to prevent recursive updates when we're managing transitions.
 * Also exposed as a store for UI to block interactions during transitions.
 */
let isTransitioning = false;
export const isPopupTransitioning = writable(false);

let transitionWatchdog: ReturnType<typeof setTimeout> | null = null;

function clearTransitionWatchdog() {
  if (transitionWatchdog !== null) {
    clearTimeout(transitionWatchdog);
    transitionWatchdog = null;
  }
}

function setTransitioning(value: boolean) {
  isTransitioning = value;
  isPopupTransitioning.set(value);
  clearTransitionWatchdog();
  if (value && typeof setTimeout !== "undefined") {
    // Auto-release the blocker if no caller clears the flag within the
    // watchdog window. Prevents a permanently unclickable UI when an
    // open/close chain is interrupted mid-transition.
    transitionWatchdog = setTimeout(() => {
      transitionWatchdog = null;
      if (isTransitioning) {
        console.warn(
          "popupManager: transition watchdog fired; forcing isTransitioning=false",
        );
        isTransitioning = false;
        isPopupTransitioning.set(false);
      }
    }, TRANSITION_WATCHDOG_MS);
  }
}

/**
 * Pending popup that should open after current transition completes.
 */
let pendingOpen: PopupState | null = null;

/**
 * Close a popup by type (triggers close animation).
 */
function closePopupByType(type: PopupType): void {
  switch (type) {
    case "lovely":
      lovelyPopupStore.set({ visible: false });
      break;
    case "uninstall":
      uninstallDialogStore.set({
        show: false,
        modName: "",
        modPath: "",
        dependents: [],
      });
      break;
    case "warning":
      showWarningPopup.update((s) => ({ ...s, visible: false }));
      break;
    case "collectionImport":
      collectionImportStore.set({ open: false, code: "" });
      break;
    case "collectionPicker":
      collectionPickerStore.set({ open: false, modTitle: null, modId: null });
      break;
    case "depPrompt":
      depPromptStore.update((s) => ({ ...s, open: false }));
      break;
    case "requires":
      requiresPopupStore.update((s) => ({ ...s, visible: false }));
      break;
    case "security":
      securityPopupStore.update((s) => ({ ...s, visible: false }));
      break;
    case "update":
      updatePopupStore.update((s) => ({ ...s, visible: false }));
      break;
    case "reportIssue":
      reportIssueStore.set({ visible: false });
      break;
    case "createBackup":
      createBackupPopupStore.set({ visible: false });
      break;
    case "restoreBackup":
      restoreBackupPopupStore.set({
        visible: false,
        backupId: "",
        backupName: "",
      });
      break;
    case "deleteBackup":
      deleteBackupPopupStore.set({
        visible: false,
        backupId: "",
        backupName: "",
      });
      break;
  }
}

/**
 * Open a popup with the given state (directly, no transition logic).
 */
function openPopupDirect(state: PopupState): void {
  setTransitioning(true);
  switch (state.type) {
    case "lovely":
      lovelyPopupStore.set(state.data as LovelyPopupState);
      break;
    case "uninstall":
      uninstallDialogStore.set(state.data as UninstallDialogState);
      break;
    case "warning":
      showWarningPopup.set(state.data as WarningPopupState);
      break;
    case "collectionImport":
      collectionImportStore.set(state.data as { open: boolean; code: string });
      break;
    case "collectionPicker":
      collectionPickerStore.set(
        state.data as {
          open: boolean;
          modTitle: string | null;
          modId: string | null;
        },
      );
      break;
    case "depPrompt":
      depPromptStore.set(state.data as DepPromptState);
      break;
    case "requires":
      requiresPopupStore.set(state.data as RequiresPopupState);
      break;
    case "security":
      securityPopupStore.set(state.data as SecurityPopupState);
      break;
    case "update":
      updatePopupStore.set(state.data as UpdatePopupState);
      break;
    case "reportIssue":
      reportIssueStore.set(state.data as ReportIssueState);
      break;
    case "createBackup":
      createBackupPopupStore.set(state.data as CreateBackupPopupState);
      break;
    case "restoreBackup":
      restoreBackupPopupStore.set(state.data as RestoreBackupPopupState);
      break;
    case "deleteBackup":
      deleteBackupPopupStore.set(state.data as DeleteBackupPopupState);
      break;
  }
  // Reset transitioning flag after a microtask to allow store update to propagate
  setTimeout(() => {
    setTransitioning(false);
  }, 0);
}

/**
 * Handle a popup wanting to open when another is already open.
 * Closes the current popup, saves it to stack, and queues the new one.
 */
function handlePopupConflict(
  currentType: PopupType,
  currentData: unknown,
  newType: PopupType,
  newData: unknown,
): void {
  // Save current popup to stack
  popupStack.update((stack) => [
    { type: currentType, data: currentData },
    ...stack,
  ]);

  // Queue the new popup
  pendingOpen = { type: newType, data: newData };

  // Close the current popup (will trigger animation)
  setTransitioning(true);
  closePopupByType(currentType);

  // After animation, open the pending popup
  setTimeout(() => {
    if (pendingOpen) {
      const toOpen = pendingOpen;
      pendingOpen = null;
      openPopupDirect(toOpen);
    }
  }, ANIMATION_DURATION);
}

/**
 * Handle a popup closing - restore previous popup from stack smoothly.
 */
function handlePopupClose(): void {
  if (isTransitioning) return;

  const stack = get(popupStack);
  if (stack.length > 0) {
    const [previous, ...rest] = stack;
    popupStack.set(rest);

    setTransitioning(true);

    // Wait for close animation to complete, then restore previous popup
    setTimeout(() => {
      openPopupDirect(previous);
    }, ANIMATION_DURATION);
  }
}

// Store previous open states for each popup type
let prevStates: Record<PopupType, { open: boolean; data: unknown }> = {
  lovely: { open: false, data: null },
  uninstall: { open: false, data: null },
  warning: { open: false, data: null },
  collectionImport: { open: false, data: null },
  collectionPicker: { open: false, data: null },
  depPrompt: { open: false, data: null },
  requires: { open: false, data: null },
  security: { open: false, data: null },
  update: { open: false, data: null },
  reportIssue: { open: false, data: null },
  createBackup: { open: false, data: null },
  restoreBackup: { open: false, data: null },
  deleteBackup: { open: false, data: null },
};

/**
 * Find which popup type is currently open (if any), excluding the given type.
 */
function findCurrentlyOpenPopup(
  excludeType: PopupType,
): { type: PopupType; data: unknown } | null {
  for (const [type, state] of Object.entries(prevStates)) {
    if (type === excludeType) continue;
    if (state.open) {
      return { type: type as PopupType, data: state.data };
    }
  }
  return null;
}

// Subscribe to each popup store to detect opens and closes
lovelyPopupStore.subscribe((state) => {
  const wasOpen = prevStates.lovely.open;
  const isOpen = state.visible;

  if (isOpen && !wasOpen && !isTransitioning) {
    // A popup is trying to open
    const currentPopup = findCurrentlyOpenPopup("lovely");
    if (currentPopup) {
      // Another popup is open - handle the conflict
      // First, immediately close this popup (it shouldn't render yet)
      lovelyPopupStore.set({ visible: false });
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "lovely",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.lovely = { open: isOpen, data: state };
});

uninstallDialogStore.subscribe((state) => {
  const wasOpen = prevStates.uninstall.open;
  const isOpen = state.show;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("uninstall");
    if (currentPopup) {
      uninstallDialogStore.set({
        show: false,
        modName: "",
        modPath: "",
        dependents: [],
      });
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "uninstall",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.uninstall = { open: isOpen, data: state };
});

showWarningPopup.subscribe((state) => {
  const wasOpen = prevStates.warning.open;
  const isOpen = state.visible;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("warning");
    if (currentPopup) {
      showWarningPopup.update((s) => ({ ...s, visible: false }));
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "warning",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.warning = { open: isOpen, data: state };
});

collectionImportStore.subscribe((state) => {
  const wasOpen = prevStates.collectionImport.open;
  const isOpen = state.open;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("collectionImport");
    if (currentPopup) {
      collectionImportStore.set({ open: false, code: "" });
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "collectionImport",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.collectionImport = { open: isOpen, data: state };
});

collectionPickerStore.subscribe((state) => {
  const wasOpen = prevStates.collectionPicker.open;
  const isOpen = state.open;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("collectionPicker");
    if (currentPopup) {
      collectionPickerStore.set({ open: false, modTitle: null, modId: null });
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "collectionPicker",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.collectionPicker = { open: isOpen, data: state };
});

depPromptStore.subscribe((state) => {
  const wasOpen = prevStates.depPrompt.open;
  const isOpen = state.open;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("depPrompt");
    if (currentPopup) {
      depPromptStore.update((s) => ({ ...s, open: false }));
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "depPrompt",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.depPrompt = { open: isOpen, data: state };
});

requiresPopupStore.subscribe((state) => {
  const wasOpen = prevStates.requires.open;
  const isOpen = state.visible;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("requires");
    if (currentPopup) {
      requiresPopupStore.update((s) => ({ ...s, visible: false }));
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "requires",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.requires = { open: isOpen, data: state };
});

securityPopupStore.subscribe((state) => {
  const wasOpen = prevStates.security.open;
  const isOpen = state.visible;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("security");
    if (currentPopup) {
      securityPopupStore.update((s) => ({ ...s, visible: false }));
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "security",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.security = { open: isOpen, data: state };
});

updatePopupStore.subscribe((state) => {
  const wasOpen = prevStates.update.open;
  const isOpen = state.visible;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("update");
    if (currentPopup) {
      updatePopupStore.update((s) => ({ ...s, visible: false }));
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "update",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.update = { open: isOpen, data: state };
});

reportIssueStore.subscribe((state) => {
  const wasOpen = prevStates.reportIssue.open;
  const isOpen = state.visible;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("reportIssue");
    if (currentPopup) {
      reportIssueStore.set({ visible: false });
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "reportIssue",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.reportIssue = { open: isOpen, data: state };
});

createBackupPopupStore.subscribe((state) => {
  const wasOpen = prevStates.createBackup.open;
  const isOpen = state.visible;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("createBackup");
    if (currentPopup) {
      createBackupPopupStore.set({ visible: false });
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "createBackup",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.createBackup = { open: isOpen, data: state };
});

restoreBackupPopupStore.subscribe((state) => {
  const wasOpen = prevStates.restoreBackup.open;
  const isOpen = state.visible;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("restoreBackup");
    if (currentPopup) {
      restoreBackupPopupStore.set({
        visible: false,
        backupId: "",
        backupName: "",
      });
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "restoreBackup",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.restoreBackup = { open: isOpen, data: state };
});

deleteBackupPopupStore.subscribe((state) => {
  const wasOpen = prevStates.deleteBackup.open;
  const isOpen = state.visible;

  if (isOpen && !wasOpen && !isTransitioning) {
    const currentPopup = findCurrentlyOpenPopup("deleteBackup");
    if (currentPopup) {
      deleteBackupPopupStore.set({
        visible: false,
        backupId: "",
        backupName: "",
      });
      handlePopupConflict(
        currentPopup.type,
        currentPopup.data,
        "deleteBackup",
        state,
      );
      return;
    }
  } else if (!isOpen && wasOpen && !isTransitioning) {
    handlePopupClose();
  }

  prevStates.deleteBackup = { open: isOpen, data: state };
});

/**
 * Clear the popup stack (use when navigating away or resetting state).
 */
export function clearPopupStack(): void {
  popupStack.set([]);
  pendingOpen = null;
  setTransitioning(false);
}

/**
 * Get the current stack depth (for debugging).
 */
export function getStackDepth(): number {
  return get(popupStack).length;
}

/**
 * Derived store that indicates if any popup is currently open.
 */
export const hasActivePopup = derived(
  [
    lovelyPopupStore,
    uninstallDialogStore,
    showWarningPopup,
    collectionImportStore,
    collectionPickerStore,
    depPromptStore,
    requiresPopupStore,
    securityPopupStore,
    updatePopupStore,
    reportIssueStore,
    createBackupPopupStore,
    restoreBackupPopupStore,
    deleteBackupPopupStore,
  ],
  ([
    $lovely,
    $uninstall,
    $warning,
    $collectionImport,
    $collectionPicker,
    $depPrompt,
    $requires,
    $security,
    $update,
    $reportIssue,
    $createBackup,
    $restoreBackup,
    $deleteBackup,
  ]) => {
    return (
      $lovely.visible ||
      $uninstall.show ||
      $warning.visible ||
      $collectionImport.open ||
      $collectionPicker.open ||
      $depPrompt.open ||
      $requires.visible ||
      $security.visible ||
      $update.visible ||
      $reportIssue.visible ||
      $createBackup.visible ||
      $restoreBackup.visible ||
      $deleteBackup.visible
    );
  },
);

/**
 * True while a managed popup is visible or any modal is mounted, global
 * shortcuts like Escape-to-go-back must stand down while this is set
 */
export const anyPopupOpen = derived(
  [hasActivePopup, modalOpen],
  ([$hasActivePopup, $modalOpen]) => $hasActivePopup || $modalOpen,
);

export function isAnyPopupOpen(): boolean {
  return get(hasActivePopup) || isModalOpen();
}
