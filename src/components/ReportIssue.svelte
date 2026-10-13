<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { Bug } from "lucide-svelte";
  import { addMessage } from "$lib/stores";
  import { invoke } from "@tauri-apps/api/core";
  import { openExternal } from "$lib/opener";
  import { reportIssueStore } from "../stores/modStore";
  import { modal } from "../utils/modalStack";

  let title = $state("");
  let description = $state("");
  const TITLE_MAX = 120;
  const DESC_MAX = 1200;
  let submitting = $state(false);

  // Error fallback modal
  let showCopyPrompt = $state(false);
  let copying = $state(false);
  let reportError = $state<string | null>(null);
  let logPreview = $state<string | null>(null);
  let logFilename = $state<string | null>(null);

  function resetForm() {
    title = "";
    description = "";
  }

  function close() {
    reportIssueStore.set({ visible: false });
    submitting = false;
  }

  function open() {
    reportIssueStore.set({ visible: true });
  }

  async function handleSubmit() {
    if (!title.trim()) {
      addMessage("Please enter a title.", "warning");
      return;
    }
    submitting = true;
    try {
      const mm_version = await invoke<string>("get_app_version");
      const url = await invoke<string>("build_issue_report_url", {
        title,
        description,
        mmVersion: mm_version,
      });
      let logCopied = false;
      try {
        const [, text] = await invoke<[string, string]>("get_latest_log");
        await navigator.clipboard.writeText(text);
        logCopied = true;
      } catch (e) {
        console.warn("Could not copy the latest log:", e);
      }
      await openExternal(url);
      addMessage(
        logCopied
          ? "Opened GitHub issue form, the log was copied to your clipboard"
          : "Opened GitHub issue form, please attach the latest log from the logs folder",
        "success",
      );
      resetForm();
      close();
    } catch (e) {
      reportError = e instanceof Error ? e.message : String(e);
      console.error("Error opening the issue form:", e);
      addMessage("Could not open the GitHub issue form", "error");
      // Offer to copy logs to clipboard
      showCopyPrompt = true;
      close();
    } finally {
      submitting = false;
    }
  }

  $effect(() => {
    if (!showCopyPrompt) {
      reportError = null;
      logPreview = null;
      logFilename = null;
    }
  });

  async function copyLogToClipboard() {
    copying = true;
    try {
      const [filename, text] = await invoke<[string, string]>("get_latest_log");
      logFilename = filename;
      logPreview = text;
      try {
        await navigator.clipboard.writeText(text);
        addMessage(`Copied ${filename} to clipboard.`, "success");
        showCopyPrompt = false;
      } catch (e) {
        console.error("Failed to copy log:", e);
        addMessage("Failed to copy log to clipboard.", "error");
      }
    } finally {
      copying = false;
    }
  }

  async function openLogsFolder() {
    try {
      const logsPath = await invoke<string>("get_logs_folder");
      await invoke("open_directory", { path: logsPath });
    } catch (e) {
      console.error("Failed to open logs folder:", e);
      addMessage("Failed to open logs folder.", "error");
    }
  }
</script>

<!-- Floating report button -->
<div class="fab-group">
  <button class="report-fab" aria-label="Report an issue" onclick={open}>
    <Bug size={16} />
  </button>
</div>

{#if $reportIssueStore.visible}
  <div
    class="modal-background"
    transition:fade={{ duration: 160 }}
    use:modal={{
      onEscape: () => {
        if (!submitting) close();
      },
    }}
  >
    <div
      class="modal"
      transition:scale={{ duration: 160, start: 0.95, opacity: 1 }}
    >
      <div class="header">
        <Bug size={28} />
        <h2>Report an issue</h2>
      </div>
      <p class="desc">
        This opens a prefilled GitHub issue with minimal hardware information.
        The latest log is copied to your clipboard so you can paste or attach it
        there.
      </p>

      <label class="input-label" for="report-title">Title</label>
      <input
        class="text-input"
        type="text"
        id="report-title"
        bind:value={title}
        maxlength={TITLE_MAX}
        placeholder="Brief summary"
      />
      <div class="counter">{title.length}/{TITLE_MAX}</div>

      <label class="input-label" for="report-description">Description</label>
      <textarea
        class="text-area default-scrollbar"
        id="report-description"
        bind:value={description}
        maxlength={DESC_MAX}
        rows={6}
        placeholder="What happened? Steps to reproduce, expected vs actual behavior, etc."
      ></textarea>
      <div class="counter">{description.length}/{DESC_MAX}</div>

      <div class="buttons">
        <button class="cancel" onclick={close} disabled={submitting}
          >Cancel</button
        >
        <button class="submit" onclick={handleSubmit} disabled={submitting}>
          {#if submitting}Opening...{:else}Open GitHub Issue{/if}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if showCopyPrompt}
  <div
    class="modal-background"
    transition:fade={{ duration: 160 }}
    use:modal={{ onEscape: () => (showCopyPrompt = false) }}
  >
    <div
      class="modal"
      transition:scale={{ duration: 160, start: 0.95, opacity: 1 }}
    >
      <h2>Opening the issue form failed</h2>
      <p class="desc">Would you like to copy the log to your clipboard?</p>
      {#if reportError}
        <p class="desc error">Reason: {reportError}</p>
      {/if}
      {#if logPreview}
        <label class="input-label" for="report-log-preview">
          {logFilename ?? "Latest log"}
        </label>
        <textarea
          id="report-log-preview"
          class="text-area default-scrollbar"
          rows={6}
          readonly>{logPreview}</textarea
        >
      {/if}
      <div class="buttons">
        <button
          class="cancel"
          onclick={() => (showCopyPrompt = false)}
          disabled={copying}>No</button
        >
        <button class="cancel" onclick={openLogsFolder} disabled={copying}
          >Open Logs Folder</button
        >
        <button class="submit" onclick={copyLogToClipboard} disabled={copying}>
          {#if copying}Copying...{:else}Copy Log{/if}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .fab-group {
    position: fixed;
    left: 1rem;
    bottom: 1rem;
    display: flex;
    gap: 0.6rem;
    align-items: center;
    z-index: 1500;
  }

  .report-fab {
    width: 36px;
    height: 36px;
    border-radius: 50%;
    border: none;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.35);
    transition:
      transform 0.15s ease,
      opacity 0.15s ease;
    background: #56a786;
    outline: 2px solid #74cca8;
    color: #fff;
  }
  .report-fab:hover {
    transform: scale(1.05);
  }
  .report-fab:active {
    transform: scale(0.98);
  }

  .modal-background {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2000; /* Above mod view overlay */
  }
  .modal {
    background: #2d2d2d;
    outline: 2px solid #74cca8;
    /* add a touch more right-side space for visual balance */
    padding: 2rem 2.5rem 2rem 2rem;
    border-radius: 8px;
    box-shadow: 0 0 15px rgba(0, 0, 0, 0.7);
    width: 520px;
    max-width: 92vw;
    color: #f4eee0;
  }
  .header {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 0.5rem;
  }
  h2 {
    font-family: "M6X11", sans-serif;
    margin: 0;
  }
  .desc {
    font-family: "M6X11", sans-serif;
    margin: 0.5rem 0 1.25rem;
  }
  .desc.error {
    color: #ffb3b3;
  }

  .input-label {
    display: block;
    margin: 0.5rem 0 0.25rem;
    font-family: "M6X11", sans-serif;
  }
  .text-input,
  .text-area {
    width: 100%;
    background: #1f1f1f;
    color: #f4eee0;
    border: 1px solid #444;
    border-radius: 6px;
    padding: 0.6rem 0.75rem;
    font-size: 1rem;
    font-family: "M6X11", sans-serif;
    outline: none;
    caret-color: #74cca8;
    box-sizing: border-box;
  }
  .text-input:focus,
  .text-input:focus-visible,
  .text-area:focus,
  .text-area:focus-visible {
    border-color: #74cca8;
    box-shadow: 0 0 0 2px rgba(116, 204, 168, 0.25);
  }
  .text-input::selection,
  .text-area::selection {
    background: #74cca8;
    color: #f4eee0; /* high-contrast text on turquoise */
  }
  .text-input::-moz-selection,
  .text-area::-moz-selection {
    background: #74cca8;
    color: #f4eee0;
  }
  .text-area {
    resize: none;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .counter {
    font-size: 0.8rem;
    color: #c9c3b7;
    text-align: right;
    margin-top: 0.25rem;
  }
  .buttons {
    margin-top: 1.25rem;
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
  }
  .cancel {
    background: #a25b5b;
    outline: #c88686 solid 2px;
    color: #fff;
    border: none;
    padding: 0.6rem 1.2rem;
    border-radius: 6px;
    cursor: pointer;
    font-family: "M6X11", sans-serif;
    font-size: 1.1rem;
    transition:
      transform 0.12s ease,
      background-color 0.12s ease,
      box-shadow 0.12s ease,
      filter 0.12s ease;
  }
  .submit {
    background: #56a786;
    outline: #74cca8 solid 2px;
    color: #fff;
    border: none;
    padding: 0.6rem 1.2rem;
    border-radius: 6px;
    cursor: pointer;
    font-family: "M6X11", sans-serif;
    font-size: 1.1rem;
    transition:
      transform 0.12s ease,
      background-color 0.12s ease,
      box-shadow 0.12s ease,
      filter 0.12s ease;
  }
  .cancel:hover {
    transform: translateY(-1px) scale(1.03);
    filter: brightness(1.05);
  }
  .submit:hover {
    transform: translateY(-1px) scale(1.03);
    filter: brightness(1.05);
  }
  .cancel:active {
    transform: translateY(0) scale(0.98);
    filter: brightness(0.95);
  }
  .submit:active {
    transform: translateY(0) scale(0.98);
    filter: brightness(0.95);
  }
  .cancel:focus-visible {
    box-shadow: 0 0 0 2px rgba(200, 134, 134, 0.35);
    outline: none;
  }
  .submit:focus-visible {
    box-shadow: 0 0 0 2px rgba(116, 204, 168, 0.35);
    outline: none;
  }
  .cancel:disabled,
  .submit:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  @media (max-width: 1160px) {
    .report-fab {
      bottom: 1rem;
      left: 0.8rem;
      width: 32px;
      height: 32px;
    }
    .modal {
      padding: 1rem 1.5rem 1rem 1rem;
    }
    /* Give inputs extra breathing room from the right edge */
    .text-input,
    .text-area {
      width: calc(100% - 0.75rem);
    }
  }
</style>
