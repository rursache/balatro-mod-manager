<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import LaunchAlertBox from "./LaunchAlertBox.svelte";
  import { addMessage } from "../lib/stores";
  import { lovelyPopupStore } from "../stores/modStore";
  import { isLinuxPlatform } from "../lib/platform";
  import { onMount } from "svelte";
  import { Check } from "lucide-svelte";
  import { launchBalatro } from "../utils/launch";

  let showAlert = false;
  let isLinux = false;
  let isLaunching = false;
  let launchViaSteam = false;
  let launchCheckTimer: ReturnType<typeof setInterval> | null = null;
  let launchTimeoutTimer: ReturnType<typeof setTimeout> | null = null;

  // Launch mode state
  let launchMode: "modded" | "vanilla" = "modded";
  let dropdownOpen = false;

  onMount(async () => {
    try {
      isLinux = await isLinuxPlatform();
    } catch (_) {
      isLinux = false;
    }
    // Load saved launch mode
    try {
      const savedMode = await invoke<string>("get_launch_mode");
      if (savedMode === "modded" || savedMode === "vanilla") {
        launchMode = savedMode;
      }
    } catch (_) {
      // Default to modded if we can't load
      launchMode = "modded";
    }
  });

  async function setMode(mode: "modded" | "vanilla") {
    const previousMode = launchMode;
    // Optimistic update - instant UI feedback
    launchMode = mode;
    dropdownOpen = false;

    // Save in background, revert on error
    try {
      await invoke("set_launch_mode", { mode });
    } catch (err) {
      launchMode = previousMode;
      addMessage(`Failed to set launch mode: ${err}`, "error");
    }
  }

  function handleDropdownToggle(event: MouseEvent) {
    event.stopPropagation();
    dropdownOpen = !dropdownOpen;
  }

  function handleClickOutside(event: MouseEvent) {
    const target = event.target as HTMLElement;
    if (!target.closest(".launch-wrapper")) {
      dropdownOpen = false;
    }
  }

  async function doLaunch() {
    const result = await launchBalatro(isLinux);
    if (!result.launched) {
      if (result.reason === "already_running") {
        addMessage("Balatro is already running", "error");
      } else {
        showAlert = true;
      }
    }
    return result;
  }

  const handleLaunch = async () => {
    if (isLaunching) return;
    isLaunching = true;
    try {
      isLinux = await isLinuxPlatform();
    } catch (_) {
      isLinux = false;
    }
    // Only warn about Lovely if launching in modded mode
    if (launchMode === "modded" && !isLinux) {
      try {
        const present = await invoke<boolean>("is_lovely_installed");
        if (!present) {
          lovelyPopupStore.set({
            visible: true,
            source: "launch",
            onLaunchAnyway: async () => {
              await doLaunch();
            },
          });
          isLaunching = false;
          return;
        }
      } catch (_) {
        // ignore detection errors, proceed
      }
    }
    let launched = false;
    let usesSteam = false;
    try {
      const res = await doLaunch();
      launched = res.launched;
      usesSteam = res.usesSteam;
    } catch (_) {
      launched = false;
    }
    if (!launched) {
      isLaunching = false;
      return;
    }
    launchViaSteam = usesSteam;

    if (launchCheckTimer) clearInterval(launchCheckTimer);
    if (launchTimeoutTimer) clearTimeout(launchTimeoutTimer);

    if (isLinux) {
      launchCheckTimer = setInterval(async () => {
        try {
          const running: boolean = await invoke("check_balatro_running");
          if (running) {
            if (launchCheckTimer) clearInterval(launchCheckTimer);
            launchCheckTimer = null;
            if (launchTimeoutTimer) clearTimeout(launchTimeoutTimer);
            launchTimeoutTimer = null;
            isLaunching = false;
            return;
          }
        } catch (_) {
          // ignore polling errors
        }
      }, 500);

      launchTimeoutTimer = setTimeout(async () => {
        if (launchCheckTimer) {
          clearInterval(launchCheckTimer);
          launchCheckTimer = null;
        }
        try {
          const running: boolean = await invoke("check_balatro_running");
          if (running) {
            isLaunching = false;
            return;
          }
        } catch (_) {
          // ignore polling errors
        }
        isLaunching = false;
        addMessage("Launch timed out. Try again.", "warning");
      }, 12000);
      return;
    }

    launchCheckTimer = setInterval(async () => {
      try {
        const running: boolean = await invoke("check_balatro_running");
        if (running) {
          if (launchCheckTimer) clearInterval(launchCheckTimer);
          launchCheckTimer = null;
          if (launchTimeoutTimer) clearTimeout(launchTimeoutTimer);
          launchTimeoutTimer = null;
          isLaunching = false;
          return;
        }
      } catch (_) {
        // ignore polling errors
      }
    }, 500);

    launchTimeoutTimer = setTimeout(async () => {
      if (launchCheckTimer) {
        clearInterval(launchCheckTimer);
        launchCheckTimer = null;
      }
      try {
        const running: boolean = await invoke("check_balatro_running");
        if (running) {
          isLaunching = false;
          return;
        }
      } catch (_) {
        // ignore polling errors
      }
      if (launchViaSteam) {
        try {
          const steamRunning: boolean = await invoke("check_steam_running");
          if (steamRunning) {
            isLaunching = false;
            return;
          }
        } catch (_) {
          // ignore polling errors
        }
      }
      isLaunching = false;
      addMessage("Launch timed out. Try again.", "warning");
    }, 12000);
  };

  onMount(() => {
    // Add click outside listener
    document.addEventListener("click", handleClickOutside);
    return () => {
      if (launchCheckTimer) clearInterval(launchCheckTimer);
      if (launchTimeoutTimer) clearTimeout(launchTimeoutTimer);
      document.removeEventListener("click", handleClickOutside);
    };
  });

  const handleAlertClose = () => {
    showAlert = false;
  };
</script>

<div class="launch-container">
  <div class="launch-wrapper">
    <button
      class="launch-button launch-main"
      onclick={handleLaunch}
      disabled={isLaunching}
    >
      {launchMode === "modded" ? "Launch" : "Launch Vanilla"}
    </button><button
      class="launch-button launch-dropdown-toggle"
      onclick={handleDropdownToggle}
      disabled={isLaunching}
      aria-label="Select launch mode"
    >
      <span class="dropdown-arrow"></span>
    </button>

    {#if dropdownOpen}
      <div class="dropdown-menu">
        <button
          class="dropdown-item"
          class:active={launchMode === "modded"}
          onclick={() => setMode("modded")}
        >
          <span class="check"
            >{#if launchMode === "modded"}<Check
                size={16}
                strokeWidth={3}
              />{/if}</span
          >
          Modded
        </button>
        <button
          class="dropdown-item"
          class:active={launchMode === "vanilla"}
          onclick={() => setMode("vanilla")}
        >
          <span class="check"
            >{#if launchMode === "vanilla"}<Check
                size={16}
                strokeWidth={3}
              />{/if}</span
          >
          Vanilla
        </button>
      </div>
    {/if}
  </div>
</div>

<LaunchAlertBox show={showAlert} onClose={handleAlertClose} />

<style>
  .launch-container {
    position: absolute;
    top: 2.5rem;
    right: 0rem;
  }

  .launch-wrapper {
    position: relative;
    display: inline-flex;
  }

  .launch-button {
    background: var(--ui-launch-bg);
    color: var(--ui-launch-text);
    font-family: "M6X11", sans-serif;
    font-size: 3.2rem;
    border: none;
    cursor: pointer;
    transition: all 0.2s ease;
    text-shadow:
      -2px -2px 0 #000,
      2px -2px 0 #000,
      -2px 2px 0 #000,
      2px 2px 0 #000;
    box-shadow: inset 0 0 10px rgba(0, 0, 0, 0.3);
  }

  .launch-main {
    padding: 0.5rem 1.6rem 0.5rem 2.2rem;
    border-radius: 8px 0 0 8px;
    outline: 3px solid var(--ui-launch-outline);
    outline-offset: -3px;
  }

  .launch-dropdown-toggle {
    padding: 0.5rem 0.6rem;
    border-radius: 0 8px 8px 0;
    outline: 3px solid var(--ui-launch-outline);
    outline-offset: -3px;
    border-left: 1px solid rgba(0, 0, 0, 0.25);
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .dropdown-arrow {
    display: block;
    width: 0;
    height: 0;
    border-left: 5px solid transparent;
    border-right: 5px solid transparent;
    border-top: 5px solid currentColor;
    filter: drop-shadow(0 1px 0 rgba(0, 0, 0, 0.5));
  }

  .launch-button:hover {
    background: var(--ui-launch-hover);
    transform: translateY(-2px);
  }

  .launch-wrapper:hover .launch-button {
    transform: translateY(-2px);
  }

  .launch-wrapper:hover .launch-button:active {
    transform: translateY(0);
  }

  .launch-button:active {
    transform: translateY(0);
  }

  .launch-button:disabled {
    opacity: 0.8;
    cursor: not-allowed;
    transform: none;
  }

  .launch-wrapper:hover .launch-button:disabled {
    transform: none;
  }

  .dropdown-menu {
    position: absolute;
    top: 100%;
    right: 0;
    margin-top: 0.5rem;
    background: var(--ui-launch-bg);
    border-radius: 8px;
    outline: 3px solid var(--ui-launch-outline);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.4);
    z-index: 100;
    min-width: 140px;
    overflow: hidden;
  }

  .dropdown-item {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    padding: 0.6rem 1rem;
    background: transparent;
    border: none;
    color: var(--ui-launch-text);
    font-family: "M6X11", sans-serif;
    font-size: 1.4rem;
    cursor: pointer;
    text-align: left;
    text-shadow:
      -1px -1px 0 #000,
      1px -1px 0 #000,
      -1px 1px 0 #000,
      1px 1px 0 #000;
    transition: background 0.15s ease;
  }

  .dropdown-item:hover {
    background: var(--ui-launch-hover);
  }

  .dropdown-item.active {
    background: rgba(255, 255, 255, 0.1);
  }

  .dropdown-item .check {
    width: 1.2rem;
    height: 16px;
    font-size: 1rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin-right: 0.3rem;
  }

  @media (max-width: 1160px) {
    .launch-button {
      font-size: 2.8rem;
      text-shadow:
        -1.8px -1.8px 0 #000,
        1.8px -1.8px 0 #000,
        -1.8px 1.8px 0 #000,
        1.8px 1.8px 0 #000;
    }
    .launch-container {
      top: 2.4rem;
    }
    .dropdown-arrow {
      border-left-width: 4px;
      border-right-width: 4px;
      border-top-width: 4px;
    }
    .dropdown-item {
      font-size: 1.2rem;
    }
  }
</style>
