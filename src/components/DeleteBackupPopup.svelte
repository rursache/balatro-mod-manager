<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { deleteBackupPopupStore } from "../stores/modStore";
  import { modal } from "../utils/modalStack";
  import { backupsStore } from "../stores/backups";
  import { addMessage } from "$lib/stores";

  let isProcessing = $state(false);

  async function handleDelete() {
    if (isProcessing) return;
    isProcessing = true;

    const backupId = $deleteBackupPopupStore.backupId;
    const success = await backupsStore.deleteBackup(backupId);

    if (success) {
      addMessage("Backup deleted", "success");
    } else if ($backupsStore.error) {
      addMessage($backupsStore.error, "error");
    }

    isProcessing = false;
    handleClose();
  }

  function handleClose() {
    deleteBackupPopupStore.set({
      visible: false,
      backupId: "",
      backupName: "",
    });
  }

  function handleEscape() {
    if (!isProcessing) handleClose();
  }
</script>

{#if $deleteBackupPopupStore.visible}
  <div
    class="dialog-overlay"
    transition:fade={{ duration: 160 }}
    use:modal={{ onEscape: handleEscape }}
  >
    <div
      class="dialog-content"
      transition:scale={{ duration: 160, start: 0.96 }}
      role="dialog"
      aria-modal="true"
    >
      <h3>Delete Backup?</h3>
      <p class="dialog-text">
        Are you sure you want to delete
        <strong>"{$deleteBackupPopupStore.backupName}"</strong>?
      </p>
      <p class="dialog-warning">This action cannot be undone.</p>
      <div class="dialog-actions">
        <button
          class="delete-button"
          onclick={handleDelete}
          disabled={isProcessing}
        >
          {isProcessing ? "Deleting..." : "Delete"}
        </button>
        <button
          class="cancel-button"
          data-autofocus
          onclick={handleClose}
          disabled={isProcessing}
        >
          Cancel
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .dialog-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.8);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
    backdrop-filter: blur(2px);
  }

  :global([data-platform="linux"]) .dialog-overlay {
    backdrop-filter: none;
    background: rgba(0, 0, 0, 0.92);
  }

  .dialog-content {
    width: 480px;
    max-width: 90vw;
    background: #393646;
    color: #f4eee0;
    border: 2px solid #f4eee0;
    border-radius: 12px;
    padding: 2rem;
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.45);
  }

  h3 {
    color: #fdcf51;
    font-size: 2.1rem;
    margin: 0 0 1rem;
    font-family: "M6X11", sans-serif;
    text-align: center;
  }

  .dialog-text {
    color: #f4eee0;
    font-size: 1.25rem;
    margin: 0 0 0.75rem;
    line-height: 1.5;
  }

  .dialog-text strong {
    color: #fdcf51;
  }

  .dialog-warning {
    color: #c14139;
    font-size: 1.15rem;
    margin: 0 0 1.5rem;
    line-height: 1.4;
  }

  .dialog-actions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
  }

  button {
    border: none;
    border-radius: 6px;
    padding: 0.9rem 1.25rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.25rem;
    cursor: pointer;
    transition:
      transform 0.2s ease,
      background-color 0.2s ease;
    box-shadow: 0 4px 0 rgba(0, 0, 0, 0.25);
  }

  button:hover:not(:disabled) {
    transform: translateY(-2px);
    box-shadow: 0 6px 0 rgba(0, 0, 0, 0.25);
  }

  button:active:not(:disabled) {
    transform: translateY(0);
    box-shadow: 0 2px 0 rgba(0, 0, 0, 0.25);
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .delete-button {
    background: #c14139;
    color: #f4eee0;
    border: 2px solid #a13029;
  }

  .delete-button:hover:not(:disabled) {
    background: #d2524a;
  }

  .cancel-button {
    background: #ea9600;
    color: #f4eee0;
    border: 2px solid #cc8400;
  }

  .cancel-button:hover:not(:disabled) {
    background: #fca800;
  }

  @media (max-width: 768px) {
    .dialog-content {
      padding: 1.5rem;
    }

    h3 {
      font-size: 1.5rem;
    }

    .dialog-text {
      font-size: 1rem;
    }

    .dialog-actions {
      grid-template-columns: 1fr;
    }
  }
</style>
