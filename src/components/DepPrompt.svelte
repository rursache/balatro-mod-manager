<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { get } from "svelte/store";
  import {
    depPromptStore,
    closeDepPrompt,
    setModInCollection,
  } from "../stores/collections";
  import {
    modsStore,
    installationStatus,
    loadingStates2,
  } from "../stores/modStore";
  import { invoke } from "@tauri-apps/api/core";
  import { addMessage } from "$lib/stores";
  import type { Mod } from "../stores/modStore";
  import { modal } from "../utils/modalStack";

  const ensureDownloadUrl = (mod: Mod): Mod => {
    if (mod.downloadURL && mod.downloadURL.trim().length > 0) return mod;
    if (mod._dirName) {
      return { ...mod, downloadURL: `index://${mod._dirName}` };
    }
    if (mod.id) {
      return { ...mod, downloadURL: `index://${mod.id}` };
    }
    return mod;
  };

  const installIfNeeded = async (mod?: Mod) => {
    if (!mod) return;
    if (get(installationStatus)[mod.title]) return;
    if (get(loadingStates2)[mod.title]) return;
    const withUrl = ensureDownloadUrl(mod);
    if (!withUrl.downloadURL) return;
    loadingStates2.update((s) => ({ ...s, [mod.title]: true }));
    const dependencies: string[] = [];
    if (mod.requires_steamodded) dependencies.push("Steamodded");
    if (mod.requires_talisman) dependencies.push("Talisman");
    const folderName = mod.folderName || mod.title.replace(/\s+/g, "");
    try {
      const installedPath = await invoke<string>("install_mod", {
        url: withUrl.downloadURL,
        folderName,
      });
      await invoke("add_installed_mod", {
        name: mod.title,
        path: installedPath,
        dependencies,
        currentVersion: mod.version || "",
      });
      installationStatus.update((s) => ({
        ...s,
        [mod.title]: true,
      }));
    } catch (error) {
      addMessage(
        `Failed to install ${mod.title}: ${
          error instanceof Error ? error.message : String(error)
        }`,
        "error",
      );
    } finally {
      loadingStates2.update((s) => ({ ...s, [mod.title]: false }));
    }
  };

  function handleDismiss() {
    closeDepPrompt();
  }

  async function handleAccept() {
    const state = get(depPromptStore);
    if (!state.open) return;

    const { collectionId, modTitle, modId, missing, isPreAddCheck } = state;
    closeDepPrompt();

    if (isPreAddCheck) {
      // Add the main mod to collection
      const mod = get(modsStore).find((m) => m.title === modTitle);
      setModInCollection(collectionId, modTitle, true, modId);
      await installIfNeeded(mod);
    }

    // Install missing dependencies and add them to collection
    for (const dep of missing) {
      const depMod = get(modsStore).find((m) => m.title === dep);
      setModInCollection(collectionId, dep, true, depMod?.id ?? null);
      await installIfNeeded(depMod);
    }
  }
</script>

{#if $depPromptStore.open}
  <div
    class="dep-overlay"
    role="button"
    tabindex="0"
    transition:fade={{ duration: 150 }}
    onpointerdown={handleDismiss}
    use:modal={{ onEscape: handleDismiss }}
  >
    <div
      class="dep-modal"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      transition:scale={{ duration: 200, start: 0.9, opacity: 0 }}
      onpointerdown={(e) => e.stopPropagation()}
    >
      <h3>
        {$depPromptStore.modTitle} requires {$depPromptStore.missing.join(
          " and ",
        )}
      </h3>
      {#if $depPromptStore.isPreAddCheck}
        <p>
          Install {$depPromptStore.missing.join(" and ")} first, then add "{$depPromptStore.modTitle}"
          to "{$depPromptStore.collectionName}"?
        </p>
      {:else}
        <p>
          Add {$depPromptStore.missing.join(" and ")} to "{$depPromptStore.collectionName}"?
        </p>
      {/if}
      <div class="dep-actions">
        <button class="cancel-btn" onclick={handleDismiss}>
          {$depPromptStore.isPreAddCheck ? "Cancel" : "No"}
        </button>
        <button class="confirm-btn" onclick={handleAccept}>
          {$depPromptStore.isPreAddCheck ? "Install & Add" : "Yes"}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .dep-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.65);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2300;
  }

  .dep-modal {
    background: #2d2d2d;
    border: 2px solid #f4eee0;
    border-radius: 12px;
    padding: 2rem;
    width: 520px;
    max-width: 90vw;
    text-align: center;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
  }

  h3 {
    margin: 0 0 1rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.5rem;
    color: #fdcf51;
  }

  p {
    margin: 0 0 1.5rem;
    font-size: 1.15rem;
    line-height: 1.6;
    color: #f4eee0;
  }

  .dep-actions {
    display: flex;
    justify-content: center;
    gap: 1rem;
  }

  .cancel-btn,
  .confirm-btn {
    padding: 0.75rem 1.5rem;
    border: 2px solid #f4eee0;
    border-radius: 6px;
    font-family: "M6X11", sans-serif;
    font-size: 1.1rem;
    cursor: pointer;
    transition: all 0.2s ease;
    box-shadow: 0 4px 0 rgba(0, 0, 0, 0.25);
  }

  .cancel-btn {
    background: #b86a2b;
    color: #f4eee0;
  }

  .cancel-btn:hover {
    background: #c97a3b;
    transform: translateY(-2px);
    box-shadow: 0 6px 0 rgba(0, 0, 0, 0.25);
  }

  .cancel-btn:active {
    transform: translateY(1px);
    box-shadow: 0 2px 0 rgba(0, 0, 0, 0.25);
  }

  .confirm-btn {
    background: #ea9600;
    color: #f4eee0;
  }

  .confirm-btn:hover {
    background: #f0a51a;
    transform: translateY(-2px);
    box-shadow: 0 6px 0 rgba(0, 0, 0, 0.25);
  }

  .confirm-btn:active {
    transform: translateY(1px);
    box-shadow: 0 2px 0 rgba(0, 0, 0, 0.25);
  }
</style>
