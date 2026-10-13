<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { onMount, onDestroy } from "svelte";
  import { openExternal } from "$lib/opener";
  import { updatePopupStore } from "../stores/modStore";
  import { APP_RELEASES_URL } from "../constants/links";

  async function handleDownload() {
    try {
      await openExternal(APP_RELEASES_URL);
    } catch (e) {
      console.error("Failed to open download page:", e);
    }
    $updatePopupStore.onClose();
    updatePopupStore.update((s) => ({ ...s, visible: false }));
  }

  function handleClose() {
    $updatePopupStore.onClose();
    updatePopupStore.update((s) => ({ ...s, visible: false }));
  }

  function handleDontShow() {
    $updatePopupStore.onDontShow();
    updatePopupStore.update((s) => ({ ...s, visible: false }));
  }

  let keyHandler: ((e: KeyboardEvent) => void) | null = null;
  onMount(() => {
    keyHandler = (e: KeyboardEvent) => {
      if (e.key === "Escape" && $updatePopupStore.visible) {
        e.preventDefault();
        handleClose();
      }
    };
    window.addEventListener("keydown", keyHandler);
  });
  onDestroy(() => {
    if (keyHandler) window.removeEventListener("keydown", keyHandler);
  });
</script>

{#if $updatePopupStore.visible}
  <div class="modal-background" transition:fade={{ duration: 160 }}>
    <div
      class="modal"
      transition:scale={{ duration: 160, start: 0.95, opacity: 1 }}
    >
      <h2>Update Available</h2>
      <p>A newer version of Balatro Mod Manager is available.</p>
      <p class="ver">
        Current: <span class="version">v{$updatePopupStore.currentVersion}</span
        >
        • Latest:
        <span class="version">v{$updatePopupStore.latestVersion}</span>
      </p>

      <div class="buttons">
        <button class="download-button" onclick={handleDownload}
          >Download</button
        >
        <button class="close-button" onclick={handleClose}>Close</button>
        <button class="dontshow-button" onclick={handleDontShow}
          >Don't show anymore</button
        >
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-background {
    position: fixed;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
  }
  .modal {
    background: #2d2d2d;
    outline: 2px solid #56a786;
    padding: 2rem;
    border-radius: 8px;
    box-shadow: 0 0 15px rgba(0, 0, 0, 0.7);
    max-width: 420px;
    width: 86%;
    text-align: center;
  }
  h2 {
    color: #56a786;
    margin-bottom: 0.75rem;
    font-family: "M6X11", sans-serif;
  }
  p,
  .ver {
    color: #f4eee0;
    font-size: 1.1rem;
    font-family: "M6X11", sans-serif;
    margin: 0.25rem 0 0.75rem 0;
  }
  .ver .version {
    color: #fdcf51;
  }
  .buttons {
    display: flex;
    justify-content: center;
    gap: 0.6rem;
    flex-wrap: wrap;
    margin-top: 1.5rem;
  }
  button {
    padding: 0.6rem 1.2rem;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    font-size: 1rem;
    transition: all 0.2s ease;
    font-family: "M6X11", sans-serif;
    color: #fff;
  }
  .download-button {
    background: #3498db;
    outline: #2980b9 solid 2px;
  }
  .close-button {
    background: #56a786;
    outline: #74cca8 solid 2px;
  }
  .dontshow-button {
    background: #c14139;
    outline: #a13029 solid 2px;
  }
  button:hover {
    opacity: 0.9;
    transform: translateY(-1px);
  }
</style>
