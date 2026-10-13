<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import { Check, Plus } from "lucide-svelte";
  import { addMessage } from "$lib/stores";
  import {
    collectionsStore,
    collectionPickerStore,
    closeCollectionPicker,
    createCollection,
    setModInCollection,
    isModInCollection,
    openDepPrompt,
  } from "../stores/collections";
  import {
    modsStore,
    installationStatus,
    loadingStates2,
    updateAvailableStore,
  } from "../stores/modStore";
  import { get } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import type { Mod } from "../stores/modStore";
  import { modal } from "../utils/modalStack";

  let newName = $state("");
  let creating = $state(false);

  const normalizeName = (name: string): string =>
    name
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]/g, "");

  const hasCollectionMod = (
    collection: { modTitles: string[]; modIds: string[] },
    title: string,
    id?: string | null,
  ) => {
    const normalizedTitle = normalizeName(title);
    if (id) {
      const normalizedId = normalizeName(id);
      if (collection.modIds.some((i) => normalizeName(i) === normalizedId))
        return true;
    }
    return collection.modTitles.some(
      (t) => normalizeName(t) === normalizedTitle,
    );
  };

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
      updateAvailableStore.update((s) => ({
        ...s,
        [mod.title]: false,
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

  const close = () => {
    closeCollectionPicker();
    newName = "";
  };

  async function handleCreate() {
    if (creating) return;
    creating = true;
    const result = createCollection(newName);
    if (!result.ok) {
      addMessage(result.error || "Failed to create collection.", "error");
      creating = false;
      return;
    }
    const modTitle = $collectionPickerStore.modTitle;
    const modId = $collectionPickerStore.modId;
    if (modTitle && result.id) {
      setModInCollection(result.id, modTitle, true, modId);
      const collection = $collectionsStore.find((c) => c.id === result.id);
      if (collection) {
        const mod = get(modsStore).find((m) => m.title === modTitle);
        await installIfNeeded(mod);
        if (mod) {
          const required: string[] = [];
          if (mod.requires_steamodded) required.push("Steamodded");
          if (mod.requires_talisman) required.push("Talisman");
          if (required.length > 0) {
            const normalizeName = (name: string) =>
              name
                .toLowerCase()
                .replace(/[^a-z0-9+]+/g, "")
                .trim();
            const resolveTitle = (name: string) => {
              const normalized = normalizeName(name);
              const match = get(modsStore).find(
                (m) => normalizeName(m.title) === normalized,
              );
              return match?.title ?? name;
            };
            const resolveMod = (name: string) => {
              const normalized = normalizeName(name);
              return get(modsStore).find(
                (m) => normalizeName(m.title) === normalized,
              );
            };
            const missing = required
              .map((req) => {
                const modMatch = resolveMod(req);
                return {
                  title: resolveTitle(req),
                  id: modMatch?.id ?? null,
                };
              })
              .filter((req) => !hasCollectionMod(collection, req.title, req.id))
              .map((req) => req.title);
            if (missing.length > 0) {
              openDepPrompt({
                collectionId: result.id,
                collectionName: collection.name,
                modTitle,
                modId,
                missing,
                isPreAddCheck: false,
              });
            }
          }
        }
      }
    }
    newName = "";
    creating = false;
  }

  async function handleToggle(id: string) {
    const modTitle = $collectionPickerStore.modTitle;
    const modId = $collectionPickerStore.modId;
    if (!modTitle) return;
    const collection = $collectionsStore.find((c) => c.id === id);
    if (!collection) return;
    const isMember = hasCollectionMod(collection, modTitle, modId);

    // If already a member, just remove it
    if (isMember) {
      setModInCollection(id, modTitle, false, modId);
      return;
    }

    // Check if mod has missing dependencies BEFORE adding
    const mod = get(modsStore).find((m) => m.title === modTitle);
    if (mod) {
      const required: string[] = [];
      if (mod.requires_steamodded) required.push("Steamodded");
      if (mod.requires_talisman) required.push("Talisman");

      if (required.length > 0) {
        // Check which dependencies are actually missing (not installed)
        const missingDeps: string[] = [];
        for (const dep of required) {
          const isInstalled = await invoke<boolean>("check_mod_installation", {
            modType: dep,
          });
          if (!isInstalled) {
            missingDeps.push(dep);
          }
        }

        if (missingDeps.length > 0) {
          // Show dependency prompt BEFORE adding to collection (uses popup manager)
          openDepPrompt({
            collectionId: id,
            collectionName: collection.name,
            modTitle,
            modId,
            missing: missingDeps,
            isPreAddCheck: true,
          });
          return;
        }
      }
    }

    // No missing dependencies, proceed with adding
    await addModToCollection(id, modTitle, modId, mod);
  }

  /** Actually add the mod to collection and install if needed */
  async function addModToCollection(
    collectionId: string,
    modTitle: string,
    modId: string | null,
    mod?: Mod,
  ) {
    const collection = $collectionsStore.find((c) => c.id === collectionId);
    if (!collection) return;

    setModInCollection(collectionId, modTitle, true, modId);
    await installIfNeeded(mod);

    // Check if dependencies should also be added to the collection
    if (!mod) return;
    const required: string[] = [];
    if (mod.requires_steamodded) required.push("Steamodded");
    if (mod.requires_talisman) required.push("Talisman");

    const normalizeNameLocal = (name: string) =>
      name
        .toLowerCase()
        .replace(/[^a-z0-9+]+/g, "")
        .trim();
    const resolveTitle = (name: string) => {
      const normalized = normalizeNameLocal(name);
      const match = get(modsStore).find(
        (m) => normalizeNameLocal(m.title) === normalized,
      );
      return match?.title ?? name;
    };
    const resolveMod = (name: string) => {
      const normalized = normalizeNameLocal(name);
      return get(modsStore).find(
        (m) => normalizeNameLocal(m.title) === normalized,
      );
    };
    const missingFromCollection = required
      .map((req) => {
        const modMatch = resolveMod(req);
        return {
          title: resolveTitle(req),
          id: modMatch?.id ?? null,
        };
      })
      .filter((req) => !hasCollectionMod(collection, req.title, req.id))
      .map((req) => req.title);

    if (missingFromCollection.length > 0) {
      openDepPrompt({
        collectionId,
        collectionName: collection.name,
        modTitle,
        modId,
        missing: missingFromCollection,
        isPreAddCheck: false,
      });
    }
  }
</script>

{#if $collectionPickerStore.open}
  <div
    class="picker-backdrop"
    transition:fade={{ duration: 160 }}
    role="button"
    tabindex="0"
    onpointerdown={close}
    use:modal={{ onEscape: close }}
    onkeydown={(e) => {
      const target = e.target as HTMLElement | null;
      if (target && target.closest("input, textarea, button, select")) return;
      if (e.key === "Enter" || e.key === " ") {
        close();
      }
    }}
  >
    <div
      class="picker-modal"
      role="dialog"
      aria-modal="true"
      tabindex="-1"
      transition:scale={{ duration: 160, start: 0.96 }}
      onpointerdown={(e) => e.stopPropagation()}
    >
      <h3>Add to collection</h3>
      <p class="subtitle">
        {$collectionPickerStore.modTitle ?? "Select a collection"}
      </p>

      <div class="create-row">
        <input
          class="text-input"
          type="text"
          placeholder="New collection name"
          bind:value={newName}
          onkeydown={(e) => e.key === "Enter" && handleCreate()}
        />
        <button class="primary" onclick={handleCreate} disabled={creating}>
          Create
        </button>
      </div>

      {#if $collectionsStore.length === 0}
        <div class="empty">No collections yet.</div>
      {:else}
        <div class="list">
          {#each $collectionsStore as col (col.id)}
            {@const isMember = hasCollectionMod(
              col,
              $collectionPickerStore.modTitle ?? "",
              $collectionPickerStore.modId,
            )}
            {@const modCount = Math.max(
              col.modTitles.length,
              col.modIds.length,
            )}
            <button class="row" onclick={() => handleToggle(col.id)}>
              <span class="icon" class:checked={isMember}>
                {#if isMember}
                  <Check size={18} strokeWidth={3} />
                {:else}
                  <Plus size={18} strokeWidth={2.5} />
                {/if}
              </span>
              <span class="name">{col.name}</span>
              <span class="count">{modCount}</span>
            </button>
          {/each}
        </div>
      {/if}

      <div class="footer">
        <button class="ghost" onclick={close}>Done</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .picker-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 2200;
  }

  .picker-modal {
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

  .create-row {
    display: flex;
    gap: 0.6rem;
    align-items: center;
    margin-bottom: 1rem;
  }

  .text-input {
    flex: 1;
    background: #1f1f1f;
    color: #f4eee0;
    border: 2px solid #f4eee0;
    border-radius: 6px;
    padding: 0.7rem 0.8rem;
    font-family: "M6X11", sans-serif;
    font-size: 1.05rem;
  }

  .text-input:focus {
    outline: none;
    border-color: #ea9600;
    box-shadow: 0 0 0 2px rgba(234, 150, 0, 0.35);
  }

  .text-input::selection {
    background: #f4eee0;
    color: #393646;
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    max-height: 300px;
    overflow-y: auto;
    scrollbar-width: none;
  }

  .list::-webkit-scrollbar {
    display: none;
  }

  .row {
    display: grid;
    grid-template-columns: 38px 1fr auto;
    align-items: center;
    gap: 0.6rem;
    background: rgba(255, 255, 255, 0.06);
    border: 2px solid rgba(244, 238, 224, 0.3);
    border-radius: 6px;
    padding: 0.7rem 0.9rem;
    cursor: pointer;
    color: inherit;
    font-family: "M6X11", sans-serif;
    text-align: left;
    transition:
      transform 0.15s ease,
      box-shadow 0.15s ease,
      background 0.15s ease;
  }

  .list > .row:first-child {
    margin-top: 0.4rem;
  }

  .row:hover {
    background: rgba(255, 255, 255, 0.12);
    transform: translateY(-2px);
    box-shadow: 0 6px 0 rgba(0, 0, 0, 0.2);
  }

  .row:active {
    transform: translateY(1px);
    box-shadow: 0 2px 0 rgba(0, 0, 0, 0.2);
  }

  .row span.checked {
    color: #74cca8;
  }

  .icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
  }

  .name {
    font-size: 1.15rem;
  }

  .count {
    font-size: 0.95rem;
    opacity: 0.75;
  }

  .empty {
    padding: 1rem 0.5rem;
    text-align: center;
    opacity: 0.8;
    font-size: 1.1rem;
  }

  .footer {
    display: flex;
    justify-content: flex-end;
    margin-top: 1rem;
  }

  .primary,
  .ghost {
    border: 2px solid #f4eee0;
    background: #ea9600;
    color: #f4eee0;
    padding: 0.65rem 1.2rem;
    border-radius: 6px;
    font-family: "M6X11", sans-serif;
    font-size: 1.05rem;
    cursor: pointer;
    transition:
      transform 0.15s ease,
      box-shadow 0.15s ease,
      background 0.15s ease;
    box-shadow: 0 4px 0 rgba(0, 0, 0, 0.25);
  }

  .ghost {
    background: #b86a2b;
    border-color: #f4eee0;
  }

  .primary:hover,
  .ghost:hover {
    transform: translateY(-2px);
    box-shadow: 0 7px 0 rgba(0, 0, 0, 0.25);
  }

  .primary:active,
  .ghost:active {
    transform: translateY(1px);
    box-shadow: 0 2px 0 rgba(0, 0, 0, 0.25);
  }

  .primary:hover {
    background: #f0a51a;
  }
</style>
