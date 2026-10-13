<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { addMessage } from "$lib/stores";
  import {
    collectionImportStore,
    closeCollectionImport,
    importCollectionCode,
  } from "../stores/collections";
  import { modal } from "../utils/modalStack";

  let importCode = $state("");

  $effect(() => {
    if ($collectionImportStore.open) {
      importCode = $collectionImportStore.code;
    }
  });

  function close() {
    closeCollectionImport();
    importCode = "";
  }

  function handleImport() {
    const result = importCollectionCode(importCode);
    if (!result.ok) {
      addMessage(result.error || "Failed to import collection.", "error");
      return;
    }
    addMessage("Collection imported.", "success");
    close();
  }
</script>

{#if $collectionImportStore.open}
  <div
    class="import-backdrop"
    transition:fade={{ duration: 160 }}
    role="button"
    tabindex="0"
    onpointerdown={close}
    use:modal={{ onEscape: close }}
  >
    <div
      class="import-modal"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      transition:scale={{ duration: 160, start: 0.96 }}
      onpointerdown={(e) => e.stopPropagation()}
    >
      <h3>Import collection</h3>
      <p class="subtitle">Paste a collection code.</p>
      <textarea
        class="import-textarea"
        placeholder="BMMCOLL1:..."
        bind:value={importCode}></textarea>
      <div class="import-actions">
        <button class="ghost neutral" type="button" onclick={close}>
          Cancel
        </button>
        <button class="primary" type="button" onclick={handleImport}>
          Import
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .import-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2400;
  }

  .import-modal {
    width: 520px;
    max-width: 92vw;
    background: #2d2d2d;
    color: #f4eee0;
    border: 2px solid #f4eee0;
    border-radius: 10px;
    padding: 1.8rem;
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.45);
  }

  h3 {
    margin: 0;
    font-family: "M6X11", sans-serif;
    font-size: 1.8rem;
  }

  .subtitle {
    margin: 0.4rem 0 1rem;
    color: rgba(244, 238, 224, 0.85);
    font-size: 1.1rem;
  }

  .import-textarea {
    width: 100%;
    box-sizing: border-box;
    min-height: 140px;
    background: #1f1f1f;
    color: #f4eee0;
    border: 2px solid #f4eee0;
    border-radius: 6px;
    padding: 0.7rem 0.8rem;
    font-family: "M6X11", sans-serif;
    font-size: 1rem;
    resize: vertical;
  }

  .import-textarea:focus {
    outline: none;
    border-color: #ea9600;
    box-shadow: 0 0 0 2px rgba(234, 150, 0, 0.35);
  }

  .import-textarea::selection {
    background: #f4eee0;
    color: #393646;
  }

  .import-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.7rem;
    margin-top: 1rem;
  }

  .primary,
  .ghost {
    border: 2px solid #f4eee0;
    background: #ea9600;
    color: #f4eee0;
    padding: 0.6rem 1rem;
    border-radius: 6px;
    font-family: "M6X11", sans-serif;
    font-size: 1rem;
    cursor: pointer;
    transition:
      transform 0.15s ease,
      box-shadow 0.15s ease,
      background 0.15s ease;
  }

  .ghost.neutral {
    background: #b86a2b;
    color: #f4eee0;
  }

  .primary:hover,
  .ghost:hover {
    transform: translateY(-2px);
    box-shadow: 0 4px 10px rgba(0, 0, 0, 0.2);
  }

  .primary:active,
  .ghost:active {
    transform: translateY(1px);
    box-shadow: none;
  }
</style>
