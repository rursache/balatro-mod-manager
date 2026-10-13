<script lang="ts">
  import { blur } from "svelte/transition";
  import MessageStack from "../components/MessageStack.svelte";
  import WarningPopup from "../components/WarningPopup.svelte";
  import {
    backgroundEnabled,
    currentModView,
    showWarningPopup,
    updatePopupStore,
  } from "../stores/modStore";
  import { darkMode } from "../stores/ui";
  import { onMount } from "svelte";
  import DragDropOverlay from "../components/DragDropOverlay.svelte";
  import DepPrompt from "../components/DepPrompt.svelte";
  import { Window } from "@tauri-apps/api/window";
  // Initialize popup manager subscriptions
  import { anyPopupOpen, isPopupTransitioning } from "../stores/popupManager";

  import "../app.css";
  import UpdateAvailablePopup from "../components/UpdateAvailablePopup.svelte";
  import RecoveryScreen from "../components/RecoveryScreen.svelte";
  import { updatePromptDisabled } from "../stores/update";
  import { APP_LATEST_RELEASE_API, APP_TAGS_API } from "../constants/links";
  import { invoke } from "@tauri-apps/api/core";
  import { startControllerNavigation } from "../utils/controllerNavigation";
  import { getTopModal } from "../utils/modalStack";
  import { isReleaseVersion, isUpdateAvailable } from "../utils/appVersion";

  const { data, children } = $props();

  let isWindows = $state(false);
  let detectedPlatform: string | null = null;
  let stopControllerNavigation: (() => void) | null = null;

  async function checkForUpdate() {
    try {
      if ($updatePromptDisabled) return;
      const cur = await invoke<string>("get_app_version");
      // Commit hash and dev builds are not comparable with releases
      if (!isReleaseVersion(cur)) return;
      let tag = "";
      const headers = { Accept: "application/vnd.github+json" };
      const relRes = await fetch(APP_LATEST_RELEASE_API, { headers });
      if (relRes.ok) {
        const release = await relRes.json();
        tag = release?.tag_name || "";
      }
      if (!tag) {
        // No published release yet, fall back to the newest tag
        const tagRes = await fetch(APP_TAGS_API, { headers });
        if (tagRes.ok) {
          const tags = await tagRes.json();
          if (Array.isArray(tags) && tags.length > 0) {
            tag = tags[0].name || "";
          }
        }
      }
      if (!tag) return;
      const latest = tag.replace(/^v/i, "");
      if (isUpdateAvailable(cur, latest)) {
        updatePopupStore.set({
          visible: true,
          currentVersion: cur,
          latestVersion: latest,
          onClose: () => {},
          onDontShow: () => {
            updatePromptDisabled.set(true);
          },
        });
      }
    } catch (e) {
      console.warn("Update check failed:", e);
    }
  }

  async function detectPlatform() {
    if (typeof navigator === "undefined") return;

    // Use UA as an immediate hint while awaiting the plugin result to avoid UI jumps
    const ua = navigator.userAgent.toLowerCase();
    if (ua.includes("windows")) {
      isWindows = true;
    }

    try {
      const { platform } = await import("@tauri-apps/plugin-os");
      detectedPlatform = await platform();
    } catch (_) {
      if (ua.includes("linux")) detectedPlatform = "linux";
      else if (ua.includes("mac")) detectedPlatform = "macos";
      else if (ua.includes("windows")) detectedPlatform = "windows";
    }

    if (detectedPlatform) {
      document.documentElement.dataset.platform = detectedPlatform;
      isWindows = detectedPlatform === "windows";
    }
  }

  /**
   * Make absolutely sure the window becomes visible. If anything else in
   * startup throws (deserialisation crash, plugin import error, network
   * hang etc.) the user must still get a window. Falls back to a single
   * deadline-based show.
   */
  let windowShown = false;
  async function forceShowWindow(reason: string) {
    if (windowShown) return;
    windowShown = true;
    try {
      const appWindow = Window.getCurrent();
      await appWindow.show();
      await appWindow.setFocus();
    } catch (e) {
      console.warn(`forceShowWindow (${reason}) failed:`, e);
    }
  }

  async function setupAppWindow() {
    try {
      // Wait for the next frame to ensure the page is painted, but never
      // longer than 1500ms. If raf never fires (rare WebView2 hang), the
      // deadline below shows the window anyway.
      await new Promise<void>((resolve) => {
        let resolved = false;
        const done = () => {
          if (resolved) return;
          resolved = true;
          resolve();
        };
        requestAnimationFrame(() => requestAnimationFrame(done));
        setTimeout(done, 1500);
      });
    } finally {
      await forceShowWindow("setupAppWindow");
    }
  }

  // Start from the focused element so B behaves exactly like a keyboard Escape
  function dispatchEscape() {
    const active = document.activeElement;
    const target =
      active instanceof HTMLElement && active !== document.body
        ? active
        : window;
    target.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Escape",
        bubbles: true,
        cancelable: true,
      }),
    );
  }

  function showExitConfirmation() {
    if ($showWarningPopup.visible) return;
    showWarningPopup.set({
      visible: true,
      message: "Exit Balatro Mod Manager?",
      onConfirm: () => {
        invoke("exit_application");
      },
      onCancel: () => {},
    });
  }

  function handleControllerCancel() {
    if ($anyPopupOpen || $currentModView) {
      dispatchEscape();
      return;
    }

    if (data.url === "/" || data.url === "/main" || data.url === "/main/") {
      showExitConfirmation();
      return;
    }

    dispatchEscape();
  }

  onMount(() => {
    const unsubscribeTheme = darkMode.subscribe((enabled) => {
      document.documentElement.dataset.theme = enabled ? "dark" : "light";
      document.documentElement.style.colorScheme = enabled ? "dark" : "light";
    });

    // Last-resort safety net: if nothing showed the window within 3 s of
    // mount, show it ourselves. Prevents the "process alive, window
    // hidden" failure mode reported on Windows.
    const safetyNet = window.setTimeout(() => {
      forceShowWindow("safety-net-timeout");
    }, 3000);

    const onUnhandledError = () => forceShowWindow("unhandled-error");
    window.addEventListener("error", onUnhandledError);
    window.addEventListener("unhandledrejection", onUnhandledError);

    detectPlatform();
    setupAppWindow();
    checkForUpdate();
    stopControllerNavigation = startControllerNavigation({
      root: () => getTopModal() ?? document,
      onCancel: handleControllerCancel,
    });

    return () => {
      unsubscribeTheme();
      stopControllerNavigation?.();
      window.clearTimeout(safetyNet);
      window.removeEventListener("error", onUnhandledError);
      window.removeEventListener("unhandledrejection", onUnhandledError);
    };
  });
</script>

<MessageStack />
<DragDropOverlay />
<DepPrompt />
{#if $isPopupTransitioning}
  <div class="popup-transition-blocker"></div>
{/if}
<div
  class="layout-container app-body"
  style:--gradient-opacity={$backgroundEnabled ? 0 : 1}
  style:--dot-size={isWindows ? "1.5px" : "0.45px"}
  style:--dot-color={isWindows ? "var(--ui-bg-dot-win)" : "var(--ui-bg-dot)"}
>
  {#key data.url}
    <div
      in:blur={{ duration: 300, delay: 150 }}
      out:blur={{ duration: 150 }}
      class="page-content"
    >
      <svelte:boundary onerror={(e) => console.error("Page boundary:", e)}>
        {@render children()}
        {#snippet failed(error)}
          <RecoveryScreen {error} />
        {/snippet}
      </svelte:boundary>
    </div>
  {/key}
</div>

<UpdateAvailablePopup />
<WarningPopup
  visible={$showWarningPopup.visible}
  message={$showWarningPopup.message}
  onConfirm={$showWarningPopup.onConfirm}
  onCancel={$showWarningPopup.onCancel}
/>

<style>
  .layout-container {
    width: 100%;
    height: 100%;
    position: fixed;
    top: 0;
    left: 0;
    overflow: hidden;
    background-color: var(--ui-bg); /* Fallback background color */
  }

  .layout-container::before {
    content: "";
    position: fixed;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    opacity: var(--gradient-opacity, 1);
    transition: opacity 0.4s cubic-bezier(0.4, 0, 0.2, 1);
    background-color: var(--ui-bg);
    background-image:
      radial-gradient(
        var(--dot-color, #d66060) var(--dot-size, 0.45px),
        transparent var(--dot-size, 0.45px)
      ),
      radial-gradient(
        var(--dot-color, #d66060) var(--dot-size, 0.45px),
        var(--ui-bg) var(--dot-size, 0.45px)
      );
    background-size: 18px 18px;
    background-position:
      0 0,
      9px 9px;
    z-index: -2; /* Adjust z-index to ensure proper layering */
    pointer-events: none; /* Ensure the background doesn't block interactions */
  }

  .page-content {
    width: 100%;
    height: 100%;
    position: relative;
    overflow: hidden;
    z-index: 1; /* Ensure content sits above the background */
  }

  @media screen and (min-width: 1920px) {
    .layout-container::before {
      background-size: 24px 24px;
      background-position:
        0 0,
        12px 12px;
    }
  }

  .popup-transition-blocker {
    position: fixed;
    inset: 0;
    z-index: 2500;
    pointer-events: auto;
  }
</style>
