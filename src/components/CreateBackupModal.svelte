<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { createBackupPopupStore } from "../stores/modStore";
  import { modal } from "../utils/modalStack";
  import { backupsStore } from "../stores/backups";
  import { addMessage } from "$lib/stores";

  let backupName = $state("");
  let isCreating = $state(false);

  async function handleCreate() {
    if (isCreating) return;
    isCreating = true;

    const backup = await backupsStore.createBackup(
      backupName.trim() || undefined,
    );
    if (backup) {
      addMessage("Backup created successfully", "success");
      handleClose();
    } else if ($backupsStore.error) {
      addMessage($backupsStore.error, "error");
    }
    isCreating = false;
  }

  function handleClose() {
    backupName = "";
    createBackupPopupStore.set({ visible: false });
  }

  function handleInputKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !isCreating) {
      e.preventDefault();
      handleCreate();
    }
  }
</script>

{#if $createBackupPopupStore.visible}
  <div
    class="modal-overlay"
    transition:fade={{ duration: 160 }}
    use:modal={{ onEscape: handleClose }}
  >
    <div
      class="modal-content"
      transition:scale={{ duration: 160, start: 0.96 }}
      role="dialog"
      aria-modal="true"
    >
      <h3>Create Backup</h3>
      <p class="subtitle">
        Give your backup a name (optional). If left empty, it will be labeled as
        "Manual backup".
      </p>

      <div class="input-group">
        <label for="backup-name">Backup Name</label>
        <input
          id="backup-name"
          class="text-input"
          type="text"
          placeholder="My backup (optional)"
          bind:value={backupName}
          onkeydown={handleInputKeydown}
        />
      </div>

      <div class="modal-actions">
        <button
          class="create-button"
          onclick={handleCreate}
          disabled={isCreating}
        >
          {isCreating ? "Creating..." : "Create"}
        </button>
        <button
          class="cancel-button"
          onclick={handleClose}
          disabled={isCreating}
        >
          Cancel
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.8);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000;
    backdrop-filter: blur(2px);
  }

  :global([data-platform="linux"]) .modal-overlay {
    backdrop-filter: none;
    background: rgba(0, 0, 0, 0.92);
  }

  .modal-content {
    width: 480px;
    max-width: 90vw;
    background: #393646;
    color: #f4eee0;
    border: 2px solid #f4eee0;
    border-radius: 10px;
    padding: 1.8rem;
    box-shadow: 0 10px 28px rgba(0, 0, 0, 0.45);
  }

  h3 {
    margin: 0;
    font-family: "M6X11", sans-serif;
    font-size: 2.1rem;
    color: #fdcf51;
  }

  .subtitle {
    margin: 0.5rem 0 1.5rem;
    color: rgba(244, 238, 224, 0.85);
    font-size: 1.2rem;
    line-height: 1.5;
  }

  .input-group {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-bottom: 1.5rem;
  }

  label {
    font-family: "M6X11", sans-serif;
    font-size: 1.25rem;
    color: #fdcf51;
  }

  .text-input {
    background: #1f1f1f;
    color: #f4eee0;
    border: 2px solid #f4eee0;
    border-radius: 6px;
    padding: 0.75rem 1rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.2rem;
  }

  .text-input:focus {
    outline: none;
    border-color: #ea9600;
    box-shadow: 0 0 0 2px rgba(234, 150, 0, 0.35);
  }

  .text-input::placeholder {
    color: rgba(244, 238, 224, 0.4);
  }

  .text-input::selection {
    background: #f4eee0;
    color: #393646;
  }

  .modal-actions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 1rem;
  }

  button {
    border: none;
    border-radius: 6px;
    padding: 0.85rem 1.5rem;
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

  .create-button {
    background: #56a786;
    color: #f4eee0;
    border: 2px solid #459373;
  }

  .create-button:hover:not(:disabled) {
    background: #67b897;
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
    .modal-content {
      padding: 1.5rem;
    }

    h3 {
      font-size: 1.6rem;
    }

    .subtitle {
      font-size: 1rem;
    }

    .text-input {
      font-size: 1rem;
      padding: 0.65rem 0.8rem;
    }

    .modal-actions {
      grid-template-columns: 1fr;
    }

    button {
      padding: 0.8rem 1rem;
      font-size: 1rem;
    }
  }
</style>
