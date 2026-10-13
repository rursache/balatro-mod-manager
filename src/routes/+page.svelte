<script lang="ts">
  import BalatroPicker from "../components/BalatroPicker.svelte";
  import ReportIssue from "../components/ReportIssue.svelte";
  import { Menu, MenuItem } from "@tauri-apps/api/menu";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, onDestroy } from "svelte";
  import { goto } from "$app/navigation";
  import { fade } from "svelte/transition";

  interface AppInitData {
    version: string;
    existing_installation: string | null;
    security_acknowledged: boolean;
    lovely_installed: boolean;
    lovely_update_available: string | null;
    launch_mode: string;
  }

  let appVersion = $state("");
  let isLoading = $state(true);

  onMount(() => {
    const init = async () => {
      // Use batched init data call to reduce IPC overhead
      try {
        const initData = await invoke<AppInitData>("get_app_init_data");
        appVersion = initData.version;

        if (initData.existing_installation) {
          try {
            const nav = goto("/main/", { replaceState: true });
            await Promise.race([
              nav,
              new Promise((_, reject) =>
                setTimeout(() => reject(new Error("nav-timeout")), 4000),
              ),
            ]);
          } catch (_) {
            // Fallback: hard navigate if SPA router stalls
            window.location.replace("/main/");
          }
          return; // Don't show picker if navigating away
        }
      } catch (error) {
        console.error("Error loading init data:", error);
      }

      // Only show picker if no existing installation found
      isLoading = false;
    };

    init();
  });

  let contextHandler: ((e: MouseEvent) => void) | null = null;
  onMount(() => {
    contextHandler = async (e: MouseEvent) => {
      e.preventDefault();
      const menuItems = [
        await MenuItem.new({ text: "Copy", action: () => {} }),
        await MenuItem.new({ text: "Paste", action: () => {} }),
      ];
      const menu = await Menu.new({ items: menuItems });
      menu.popup();
    };
    window.addEventListener("contextmenu", contextHandler);
  });

  onDestroy(() => {
    if (contextHandler) {
      window.removeEventListener("contextmenu", contextHandler);
      contextHandler = null;
    }
  });
</script>

<div class="area">
  <ul class="circles">
    <li></li>
    <li></li>
    <li></li>
    <li></li>
    <li></li>
    <li></li>
    <li></li>
    <li></li>
    <li></li>
    <li></li>
  </ul>
</div>

{#if isLoading}
  <div class="loading-screen" transition:fade={{ duration: 400 }}>
    <div class="spinner-container">
      <div class="spinner-ring outer"></div>
      <div class="spinner-ring inner"></div>
      <div class="spinner-dot"></div>
    </div>
  </div>
{:else}
  <div class="app" transition:fade={{ duration: 300 }}>
    <h1>Welcome to Balatro Mod Manager</h1>
    <BalatroPicker />
    <ReportIssue />
    {#if appVersion}<div class="version-text">v{appVersion}</div>{/if}
  </div>
{/if}

<style>
  .area {
    background: #10122d;
    background: -webkit-linear-gradient(to left, #1a1e3c, #0d0f24);
    width: 100%;
    height: 100vh;
    position: fixed;
    top: 0;
    left: 0;
    z-index: -1;
  }

  .circles {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    overflow: hidden;
    margin: 0;
    padding: 0;
  }

  .circles li {
    position: absolute;
    display: block;
    list-style: none;
    width: 20px;
    height: 20px;
    background: rgba(255, 255, 255, 0.1);
    animation: animate 25s linear infinite;
    bottom: -150px;
  }

  .circles li:nth-child(1) {
    left: 25%;
    width: 80px;
    height: 80px;
    animation-delay: 0s;
  }

  .circles li:nth-child(2) {
    left: 10%;
    width: 20px;
    height: 20px;
    animation-delay: 2s;
    animation-duration: 12s;
  }

  .circles li:nth-child(3) {
    left: 70%;
    width: 20px;
    height: 20px;
    animation-delay: 4s;
  }

  .circles li:nth-child(4) {
    left: 40%;
    width: 60px;
    height: 60px;
    animation-delay: 0s;
    animation-duration: 18s;
  }

  .circles li:nth-child(5) {
    left: 65%;
    width: 20px;
    height: 20px;
    animation-delay: 0s;
  }

  .circles li:nth-child(6) {
    left: 75%;
    width: 110px;
    height: 110px;
    animation-delay: 3s;
  }

  .circles li:nth-child(7) {
    left: 35%;
    width: 150px;
    height: 150px;
    animation-delay: 7s;
  }

  .circles li:nth-child(8) {
    left: 50%;
    width: 25px;
    height: 25px;
    animation-delay: 15s;
    animation-duration: 45s;
  }

  .circles li:nth-child(9) {
    left: 20%;
    width: 15px;
    height: 15px;
    animation-delay: 2s;
    animation-duration: 35s;
  }

  .circles li:nth-child(10) {
    left: 85%;
    width: 150px;
    height: 150px;
    animation-delay: 0s;
    animation-duration: 11s;
  }

  @keyframes animate {
    0% {
      transform: translateY(0) rotate(0deg);
      opacity: 1;
      border-radius: 0;
    }
    100% {
      transform: translateY(-1000px) rotate(720deg);
      opacity: 0;
      border-radius: 50%;
    }
  }

  .app {
    width: 100vw;
    height: 100vh;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 2rem;
    box-sizing: border-box;
    overflow: hidden;
    position: fixed;
    top: 0;
    left: 0;
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }

  h1 {
    color: #f4eee0;
    font-size: 3.5rem;
    margin-bottom: 3rem;
    font-family: "M6X11", sans-serif;
    text-shadow:
      -2px -2px 0 #000,
      2px -2px 0 #000,
      -2px 2px 0 #000,
      2px 2px 0 #000;
  }

  :global(html) {
    overflow: hidden;
    position: fixed;
    width: 100%;
    height: 100%;
  }

  :root {
    font-family: "M6X11", sans-serif;
    font-size: 1rem;
    line-height: 24px;
    font-weight: 400;
    color: var(--text-primary);
    font-synthesis: none;
    text-rendering: optimizeLegibility;
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
    -webkit-text-size-adjust: 100%;
    -webkit-user-select: none;
    user-select: none;
    cursor: default;
    text-shadow:
      -1px -1px 0 #000,
      1px -1px 0 #000,
      -1px 1px 0 #000,
      1px 1px 0 #000;
  }

  .version-text {
    position: fixed;
    bottom: 1rem;
    right: 1rem;
    color: #f4eee0;
    font-size: 1rem;
    opacity: 0.8;
    font-family: "M6X11", sans-serif;
    text-shadow:
      -1px -1px 0 #000,
      1px -1px 0 #000,
      -1px 1px 0 #000,
      1px 1px 0 #000;
  }

  .loading-screen {
    width: 100vw;
    height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    position: fixed;
    top: 0;
    left: 0;
    z-index: 10;
  }

  .spinner-container {
    position: relative;
    width: 64px;
    height: 64px;
  }

  .spinner-ring {
    position: absolute;
    border-radius: 50%;
    border: 3px solid transparent;
  }

  .spinner-ring.outer {
    width: 100%;
    height: 100%;
    top: 0;
    left: 0;
    border-top-color: #fdcf51;
    border-right-color: #fdcf51;
    animation: spin-outer 1s cubic-bezier(0.5, 0, 0.5, 1) infinite;
  }

  .spinner-ring.inner {
    width: 70%;
    height: 70%;
    top: 15%;
    left: 15%;
    border-bottom-color: #ea9600;
    border-left-color: #ea9600;
    animation: spin-inner 0.8s cubic-bezier(0.5, 0, 0.5, 1) infinite;
  }

  .spinner-dot {
    position: absolute;
    width: 10px;
    height: 10px;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    background: #f4eee0;
    border-radius: 50%;
    animation: pulse 1s ease-in-out infinite;
  }

  @keyframes spin-outer {
    0% {
      transform: rotate(0deg);
    }
    100% {
      transform: rotate(360deg);
    }
  }

  @keyframes spin-inner {
    0% {
      transform: rotate(0deg);
    }
    100% {
      transform: rotate(-360deg);
    }
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 0.4;
      transform: translate(-50%, -50%) scale(0.8);
    }
    50% {
      opacity: 1;
      transform: translate(-50%, -50%) scale(1);
    }
  }

  :global(body) {
    margin: 0;
    padding: 0;
    overflow: hidden;
    position: fixed;
    width: 100%;
    height: 100%;
  }
</style>
