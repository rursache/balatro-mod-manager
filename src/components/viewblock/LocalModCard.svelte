<script lang="ts">
  import {
    Trash2,
    ArrowDownToLine,
    CornerDownRight,
    Folder,
    Check,
  } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { addMessage } from "$lib/stores";
  import { modsStore } from "../../stores/modStore";
  import {
    modEnabledStore,
    selectedModsStore,
    toggleModSelection,
  } from "../../stores/modStore";
  import { cardScale, darkMode } from "../../stores/ui";
  import { pickDarkPalette } from "../../utils/cardPalette";

  import type { LocalMod } from "../../stores/modStore";

  type CatalogMatch = { title: string; download_url: string; version?: string };
  type LocalModWithCatalog = LocalMod & { catalog_match?: CatalogMatch };

  interface Props {
    mod: LocalModWithCatalog;
    onUninstall: (mod: LocalModWithCatalog) => void;
    onToggleEnabled?: () => Promise<void>;
    showCheckbox?: boolean;
  }

  const {
    mod,
    onUninstall,
    onToggleEnabled = undefined,
    showCheckbox = false,
  }: Props = $props();

  // Local state for loading and enabled status
  let isInstalling = $state(false);
  let isToggling = $state(false);
  let isEnabled = $state(true); // Default to enabled if not yet checked
  let isSelected = $derived($selectedModsStore.has(mod.path));

  function handleCheckboxClick(e: Event) {
    e.stopPropagation();
    toggleModSelection(mod.path);
  }

  // Sync local isEnabled state with the store
  $effect(() => {
    const storeValue = $modEnabledStore[mod.name];
    if (storeValue !== undefined) {
      isEnabled = storeValue;
    }
  });

  async function toggleModEnabled(e: Event) {
    e.stopPropagation();
    try {
      const currentState = $modEnabledStore[mod.name] ?? isEnabled;
      const newState = !currentState;

      // Use the full path for local mods instead of just the name
      await invoke("toggle_mod_enabled_by_path", {
        modPath: mod.path,
        enabled: newState,
      });

      // Update both the store and local variable
      modEnabledStore.update((enabledMods) => ({
        ...enabledMods,
        [mod.name]: newState,
      }));
      isEnabled = newState;

      // Call the parent callback to update the filtered lists
      if (onToggleEnabled) {
        await onToggleEnabled();
      }
    } catch (error) {
      console.error(`Failed to toggle mod ${mod.name} enabled state:`, error);
    }
  }
  // Function to open the mod's directory
  async function openModDirectory(e: Event) {
    e.stopPropagation();

    try {
      // Check if path exists first
      const pathExists = await invoke("path_exists", { path: mod.path });

      if (!pathExists) {
        addMessage(`Directory not found: ${mod.path}`, "error");
        return;
      }

      // Use the custom command to open the directory
      await invoke("open_directory", { path: mod.path });
    } catch (error) {
      addMessage(`Failed to open directory: ${error}`, "error");
    }
  }

  async function installOfficialVersion(e: Event) {
    e.stopPropagation();
    const catalogMatch = mod.catalog_match;
    if (!catalogMatch) return;

    try {
      isInstalling = true;

      // Create a simplified version of the catalog mod for installation
      const catalogMod = {
        title: catalogMatch.title,
        downloadURL: catalogMatch.download_url,
        version: catalogMatch.version || "",
        requires_steamodded: false,
        requires_talisman: false,
      };

      // Find the full catalog mod if available
      const fullCatalogMod = $modsStore.find(
        (m) => m.title.toLowerCase() === catalogMatch.title.toLowerCase(),
      );

      // Use the dependency info from the full catalog mod if found
      const dependencies = [];
      if (fullCatalogMod) {
        if (fullCatalogMod.requires_steamodded) dependencies.push("Steamodded");
        if (fullCatalogMod.requires_talisman) dependencies.push("Talisman");
      }

      // Save the local mod path for later removal
      const localModPath = mod.path;

      // Create a temporary copy of the local mod in case installation fails
      await invoke("backup_local_mod", { path: localModPath });

      try {
        // Install the mod
        const installedPath: string = await invoke("install_mod", {
          url: catalogMod.downloadURL,
          folderName:
            fullCatalogMod?.folderName || catalogMod.title.replace(/\s+/g, ""),
        });

        // Verify the installed path exists before proceeding
        const pathExists = await invoke("path_exists", {
          path: installedPath,
        });
        if (!pathExists) {
          throw new Error(`Failed to verify installed path: ${installedPath}`);
        }

        // Add to database
        await invoke("add_installed_mod", {
          name: catalogMod.title,
          path: installedPath,
          dependencies,
          currentVersion: catalogMod.version,
        });

        // Now check if we should delete the local mod
        const localPathExists = await invoke("path_exists", {
          path: localModPath,
        });

        // Normalize paths for comparison (especially important for Windows)
        const normalizedLocal = localModPath.toLowerCase().replace(/\\/g, "/");
        const normalizedInstalled = installedPath
          .toLowerCase()
          .replace(/\\/g, "/");

        // Only delete the local mod if:
        // 1. It exists
        // 2. It's not the same as the installed path
        // 3. It's not a parent directory of the installed path
        if (
          localPathExists &&
          normalizedLocal !== normalizedInstalled &&
          !normalizedInstalled.startsWith(normalizedLocal + "/")
        ) {
          await invoke("delete_manual_mod", {
            path: localModPath,
          });
        }

        addMessage(
          `Installed official version of ${catalogMatch.title}`,
          "success",
        );

        // Clean up the backup
        await invoke("remove_backup", { path: localModPath });

        // Refresh the view
        onUninstall(mod);
      } catch (error) {
        // If installation failed, restore from backup
        await invoke("restore_from_backup", { path: localModPath });
        throw error;
      }
    } catch (error) {
      addMessage(`Failed to install official version: ${error}`, "error");
    } finally {
      isInstalling = false;
    }
  }

  async function uninstallMod(e: Event) {
    e.stopPropagation(); // Prevent card click if we have one
    try {
      await invoke("delete_manual_mod", {
        path: mod.path,
      });

      addMessage(`Removed ${mod.name}`, "success");
      onUninstall(mod);
    } catch (error) {
      addMessage(`Failed to remove mod: ${error}`, "error");
    }
  }

  const lightColorPairs = [
    { color1: "#4f6367", color2: "#425556" },
    { color1: "#AA778D", color2: "#906577" },
    { color1: "#A2615E", color2: "#89534F" },
    { color1: "#A48447", color2: "#8B703C" },
    { color1: "#4F7869", color2: "#436659" },
    { color1: "#728DBF", color2: "#6177A3" },
    { color1: "#5D5E8F", color2: "#4F4F78" },
    { color1: "#796E9E", color2: "#655D86" },
    { color1: "#64825D", color2: "#556E4E" },
    { color1: "#86A367", color2: "#728A57" },
    { color1: "#748C8A", color2: "#627775" },
  ];

  function pickLightRandomPair() {
    return lightColorPairs[Math.floor(Math.random() * lightColorPairs.length)];
  }

  const lightPair = pickLightRandomPair();
  let cardColors = $derived($darkMode ? pickDarkPalette(mod.name) : lightPair);

  const hasNewerVersion = $derived(
    !!(
      mod.catalog_match &&
      mod.catalog_match.version &&
      mod.version &&
      mod.catalog_match.version !== mod.version
    ),
  );
</script>

<div
  class="mod-card"
  class:compact={$cardScale <= 0.85}
  class:selected={isSelected}
  style="--bg-color: {cardColors.color1}; --bg-color-2: {cardColors.color2};"
>
  <!-- Dark overlay to improve readability -->
  <div class="overlay"></div>

  {#if showCheckbox}
    <button
      class="select-checkbox"
      class:checked={isSelected}
      onclick={handleCheckboxClick}
      title={isSelected ? "Deselect" : "Select"}
    >
      {#if isSelected}
        <Check size={14} color="white" />
      {/if}
    </button>
  {/if}

  {#if mod.catalog_match}
    <div class="catalog-badge">
      <CornerDownRight size={14} />
      <span>In Catalog</span>
    </div>
  {/if}

  <div class="mod-content">
    <h3>{mod.name}</h3>
    <p class="description">{mod.description}</p>

    <div class="mod-meta">
      <div class="author">
        <span>By: {mod.author.join(", ")}</span>
      </div>
      {#if mod.version}
        <div class="mod-version">
          <span>Version: {mod.version}</span>
        </div>
      {/if}

      {#if mod.catalog_match && hasNewerVersion}
        <div class="catalog-version">
          <span>Catalog version: {mod.catalog_match.version}</span>
        </div>
      {/if}
    </div>
  </div>

  <div class="button-container">
    {#if mod.catalog_match}
      <button
        class="folder-button"
        title="Open mod directory"
        onclick={openModDirectory}
      >
        <Folder size={18} />
      </button>
      <!-- Enable/Disable toggle button -->
      <button
        class="toggle-button"
        class:enabled={$modEnabledStore[mod.name] ?? isEnabled}
        class:disabled={!($modEnabledStore[mod.name] ?? isEnabled)}
        title={($modEnabledStore[mod.name] ?? isEnabled)
          ? "Disable Mod"
          : "Enable Mod"}
        onclick={toggleModEnabled}
      >
        {#if $modEnabledStore[mod.name] ?? isEnabled}
          ON
        {:else}
          OFF
        {/if}
      </button>
      <button
        class="install-button"
        title="Install official version"
        onclick={installOfficialVersion}
        disabled={isInstalling}
      >
        {#if isInstalling}
          <div class="spinner"></div>
        {:else}
          <ArrowDownToLine size={15} />
          <span class="smaller-text">Get Official</span>
        {/if}
      </button>
      <button class="trash-button" title="Remove Mod" onclick={uninstallMod}>
        <Trash2 size={18} />
      </button>
    {:else}
      <button
        class="folder-button"
        title="Open mod directory"
        onclick={openModDirectory}
      >
        <Folder size={18} />
      </button>
      <!-- Added toggle button here -->
      <button
        class="toggle-button"
        class:enabled={$modEnabledStore[mod.name] ?? isEnabled}
        class:disabled={!($modEnabledStore[mod.name] ?? isEnabled)}
        title={($modEnabledStore[mod.name] ?? isEnabled)
          ? "Disable Mod"
          : "Enable Mod"}
        onclick={toggleModEnabled}
      >
        {#if $modEnabledStore[mod.name] ?? isEnabled}
          ON
        {:else}
          OFF
        {/if}
      </button>
      <button class="delete-button" title="Remove Mod" onclick={uninstallMod}>
        <Trash2 size={18} />
        Remove
      </button>
    {/if}
  </div>
</div>

<style>
  .mod-card {
    --bg-color: var(--bg-color, #4f6367);
    --bg-color-2: var(--bg-color-2, #334461);

    display: flex;
    flex-direction: column;
    position: relative;
    border-radius: 8px;
    overflow: hidden;
    border: 2px solid var(--ui-mod-panel-border);
    width: calc(300px * var(--card-scale, 1));
    max-width: calc(300px * var(--card-scale, 1));
    height: calc(330px * var(--card-scale, 1));
    content-visibility: auto;
    contain-intrinsic-size: 300px 330px;
    margin: 0 auto;
    padding: 1rem;
    box-sizing: border-box;
    background-size: 100% 200%;
    transition: all 0.3s ease;
    background-image: repeating-linear-gradient(
      -45deg,
      var(--bg-color),
      var(--bg-color) 10px,
      var(--bg-color-2) 10px,
      var(--bg-color-2) 20px
    );
  }

  /* Dark overlay for better text readability */
  .overlay {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background-color: var(--ui-card-overlay);
    backdrop-filter: blur(1px);
    -webkit-backdrop-filter: blur(1px);
    z-index: 1;
  }

  :global([data-platform="linux"]) .overlay {
    backdrop-filter: none;
    -webkit-backdrop-filter: none;
    background-color: var(--ui-card-overlay-strong);
  }

  .catalog-badge {
    position: absolute;
    top: 1rem;
    right: 1rem;
    background: var(--ui-success);
    color: var(--ui-text);
    padding: 0.3rem 0.5rem;
    border-radius: 4px;
    font-size: 0.9rem;
    display: flex;
    align-items: center;
    gap: 0.3rem;
    z-index: 3;
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.2);
    border: 1px solid var(--ui-glass-border);
  }

  .mod-card:hover {
    animation: stripe-slide-up 1.5s linear infinite;
    scale: 1.05;
  }

  :global([data-platform="linux"]) .mod-card:hover {
    animation: none;
  }

  @keyframes stripe-slide-up {
    0% {
      background-position: 0 0;
    }
    100% {
      background-position: 0 -55px;
    }
  }

  .mod-content {
    flex: 1;
    padding: 0.5rem;
    position: relative;
    z-index: 2;
    /* Reserve space for buttons at bottom */
    padding-bottom: calc(50px * var(--card-scale, 1));
    overflow: hidden;
  }

  h3 {
    color: var(--ui-heading);
    font-size: calc(1.5rem * var(--card-scale, 1));
    margin-bottom: 0;
    text-shadow: 1px 1px 2px rgba(0, 0, 0, 0.5);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Compact cards: clamp title to 1 line only when compact */
  .mod-card.compact h3 {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 1;
    line-clamp: 1;
    overflow: hidden;
    word-break: break-word;
    overflow-wrap: anywhere;
    text-overflow: ellipsis;
  }

  .description {
    color: var(--ui-text);
    font-size: 1.1rem;
    line-height: 1.3;
    margin: calc(0.3rem * var(--card-scale, 1)) 0 1rem;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    padding: 0 0.1rem;
    word-break: break-word;
    overflow-wrap: anywhere;
    text-overflow: ellipsis;
  }

  .mod-card.compact .description {
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .mod-card.compact .mod-meta {
    margin-bottom: 0.5rem;
  }

  .mod-card.compact .mod-version,
  .mod-card.compact .catalog-version {
    margin-top: 0.2rem;
  }

  .mod-meta {
    font-size: 1rem;
    color: var(--ui-text);
    margin-bottom: 1rem;
  }

  .author {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 1;
    line-clamp: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .mod-version,
  .catalog-version {
    margin-top: 0.3rem;
  }

  .catalog-version {
    color: var(--ui-success);
    font-weight: bold;
  }

  /* Button container styling */
  .button-container {
    display: flex;
    gap: 0.35rem;
    position: absolute;
    bottom: 1rem;
    left: 1rem;
    width: calc(100% - 2rem);
    z-index: 2;
  }

  /* Button styles */
  .delete-button {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.4rem;
    padding: calc(0.6rem * var(--card-scale, 1));
    color: var(--ui-text);
    border: none;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.2s ease;
    font-family: "M6X11", sans-serif;
    font-size: calc(0.95rem * var(--card-scale, 1));
    min-height: calc(38px * var(--card-scale, 1));
    position: relative;
  }

  .install-button {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: calc(0.3rem * var(--card-scale, 1));
    padding: calc(0.6rem * var(--card-scale, 1))
      calc(0.4rem * var(--card-scale, 1));
    color: var(--ui-text);
    border: none;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.2s ease;
    font-family: "M6X11", sans-serif;
    min-height: calc(38px * var(--card-scale, 1));
    position: relative;
    background: var(--ui-success);
    outline: var(--ui-button-green-border) solid 2px;
  }

  .delete-button {
    background: var(--ui-danger);
    outline: var(--ui-danger-outline) solid 2px;
  }

  .delete-button:hover {
    background: var(--ui-danger-hover);
    transform: translateY(-2px);
  }

  .delete-button:active,
  .install-button:active {
    transform: translateY(1px);
  }

  .install-button:not(:disabled):active {
    transform: translateY(1px);
    background: var(--ui-success-hover);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  }

  .install-button:disabled {
    opacity: 0.8;
    cursor: not-allowed;
  }

  .folder-button {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: calc(36px * var(--card-scale, 1));
    height: calc(38px * var(--card-scale, 1));
    padding: calc(6px * var(--card-scale, 1));
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.2s ease;
    background-color: var(--ui-success-alt);
    color: var(--ui-text);
    border: none;
    outline: var(--ui-button-forest-border) solid 2px;
    flex-shrink: 0;
  }

  .folder-button:hover {
    background-color: var(--ui-success-alt-hover);
    transform: translateY(-2px);
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.2);
  }

  .folder-button:active {
    transform: translateY(1px);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  }

  /* Toggle button styles */
  .toggle-button {
    display: flex;
    align-items: center;
    justify-content: center;
    /* Fixed width to ensure ON/OFF buttons are same size */
    min-width: calc(44px * var(--card-scale, 1));
    height: calc(38px * var(--card-scale, 1));
    padding: calc(6px * var(--card-scale, 1)) calc(8px * var(--card-scale, 1));
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.2s ease;
    color: var(--ui-text);
    border: none;
    flex-shrink: 0;
    font-family: "M6X11", sans-serif;
    font-size: calc(1rem * var(--card-scale, 1));
  }

  .toggle-button.enabled {
    background: var(--ui-success-strong); /* Bright green when enabled */
    outline: var(--ui-success-strong) solid 2px;
  }

  .toggle-button.disabled {
    background: var(--ui-neutral); /* Gray when disabled, instead of red */
    outline: var(--ui-neutral-outline) solid 2px;
  }

  .toggle-button:hover.enabled {
    background: var(--ui-success-strong-hover); /* Lighter green on hover */
    transform: translateY(-2px);
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.2);
  }

  .toggle-button:hover.disabled {
    background: var(--ui-neutral-hover); /* Lighter gray on hover */
    transform: translateY(-2px);
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.2);
  }

  .toggle-button:active {
    transform: translateY(1px);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  }

  .trash-button {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: calc(36px * var(--card-scale, 1));
    height: calc(38px * var(--card-scale, 1));
    padding: calc(6px * var(--card-scale, 1));
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.2s ease;
    background-color: var(--ui-danger);
    color: var(--ui-text);
    border: none;
    outline: var(--ui-danger-outline) solid 2px;
    flex-shrink: 0;
  }

  .trash-button:hover {
    background-color: var(--ui-danger-hover);
    transform: translateY(-2px);
    box-shadow: 0 2px 4px rgba(0, 0, 0, 0.2);
  }

  .trash-button:active {
    transform: translateY(1px);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
  }

  .install-button:hover:not(:disabled) {
    background: var(--ui-success-hover);
    transform: translateY(-2px);
    box-shadow: 0 4px 6px rgba(0, 0, 0, 0.15);
  }

  .smaller-text {
    font-size: 0.8rem;
    font-weight: 500;
  }

  /* Spinner for loading state */
  .spinner {
    border: 2px solid var(--ui-glass-border);
    border-top: 2px solid var(--ui-text);
    border-radius: 50%;
    width: calc(16px * var(--card-scale, 1));
    height: calc(16px * var(--card-scale, 1));
    animation: spin 1s linear infinite;
    margin: 0 auto;
    display: inline-block;
  }

  @keyframes spin {
    0% {
      transform: rotate(0deg);
    }
    100% {
      transform: rotate(360deg);
    }
  }

  @media (max-width: 1160px) {
    .mod-card {
      width: 100%;
    }
  }

  /* Selection checkbox */
  .select-checkbox {
    position: absolute;
    top: 0.5rem;
    left: 0.5rem;
    width: 24px;
    height: 24px;
    border-radius: 4px;
    border: 2px solid var(--ui-mod-panel-border);
    /* Opaque so the thumbnail under the corner never shows through */
    background: #262626;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
    cursor: pointer;
    z-index: 10;
    display: flex;
    align-items: center;
    justify-content: center;
    transition:
      background 0.15s ease,
      border-color 0.15s ease;
    padding: 0;
  }

  .select-checkbox:hover {
    background: #3a3a3a;
  }

  .select-checkbox.checked {
    background: var(--ui-success-strong);
  }

  .select-checkbox.checked:hover {
    background: var(--ui-success-strong-hover);
  }

  .mod-card.selected {
    outline: 2px solid var(--ui-success);
    outline-offset: -2px;
  }
</style>
