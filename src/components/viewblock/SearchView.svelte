<script lang="ts">
  import type { InstalledMod, Mod } from "../../stores/modStore";
  import { onMount } from "svelte";
  import {
    installationStatus,
    modsStore,
    loadingStates2 as loadingStates,
    uninstallDialogStore,
  } from "../../stores/modStore";
  // lightweight debounce to avoid pulling in lodash for a single helper
  function debounce<TArgs extends unknown[]>(
    fn: (...args: TArgs) => void,
    wait: number,
  ) {
    let t: ReturnType<typeof setTimeout> | null = null;
    return (...args: TArgs) => {
      if (t) clearTimeout(t);
      t = setTimeout(() => fn(...args), wait);
    };
  }
  import { fuzzySearch } from "../../utils/fuzzy";
  import { currentModView } from "../../stores/modStore";
  import { invoke } from "@tauri-apps/api/core";
  import { fetchCachedMods } from "../../stores/modCache";
  import { addMessage } from "$lib/stores";
  import { fade } from "svelte/transition";
  import ModCard from "./ModCard.svelte";
  import {
    collectionsStore,
    activeCollectionIds,
    removeActiveCollection,
  } from "../../stores/collections";
  import { get } from "svelte/store";

  let searchQuery = $state("");
  let searchResults = $state<Mod[]>([]);
  let isSearching = $state(false);
  let mods = $state<Mod[]>([]);
  // Cached searchable haystack, parallel to `mods`. Rebuilt when the catalog changes.
  let haystack: string[] = [];
  let lastModCount = 0; // Track mod count to avoid unnecessary rebuilds
  let installedMods = $state<InstalledMod[]>([]);
  let mod = $state<Mod | null>(null);
  let searchInput: HTMLInputElement;

  function handleModClick(mod: Mod) {
    currentModView.set(mod);
  }

  const { onCheckDependencies } = $props<{
    onCheckDependencies?: (
      requirements: { steamodded: boolean; talisman: boolean },
      downloadAction: () => Promise<void>,
    ) => void;
  }>();

  const getAllInstalledMods = async () => {
    try {
      const installed = await fetchCachedMods();
      installedMods = installed.map((mod) => ({
        name: mod.name,
        path: mod.path,
      }));
    } catch (error) {
      console.error("Failed to get installed mods:", error);
    }
  };

  const uninstallMod = async (mod: Mod) => {
    const isCoreMod = ["steamodded", "talisman"].includes(
      mod.title.toLowerCase(),
    );

    try {
      await getAllInstalledMods();
      const installedMod = installedMods.find(
        (m) => m.name.toLowerCase() === mod.title.toLowerCase(),
      );

      if (isCoreMod) {
        // Get dependents
        const dependents = await invoke<string[]>("get_dependents", {
          modName: mod.title,
        });

        // Always show dialog for core mods, even if no dependents
        uninstallDialogStore.set({
          show: true,
          modName: mod.title,
          // Path may be resolved in the dialog if missing
          modPath: installedMod?.path || "",
          dependents,
        });
      } else {
        // Immediate uninstall for normal mods
        if (!installedMod) {
          console.error("Mod not found in installed mods");
          return;
        }
        await invoke("remove_installed_mod", {
          name: mod.title,
          path: installedMod.path,
        });
        installationStatus.update((s) => ({
          ...s,
          [mod.title]: false,
        }));

        // Deactivate any active collections that contain this mod
        const collections = get(collectionsStore);
        const activeIds = get(activeCollectionIds);
        for (const id of activeIds) {
          const collection = collections.find((c) => c.id === id);
          if (collection && collection.modTitles.includes(mod.title)) {
            removeActiveCollection(id);
          }
        }
      }
    } catch (error) {
      console.error("Uninstall failed:", error);
    }
  };

  const installMod = async (mod: Mod) => {
    // Guard: don't allow re-entrancy while already loading
    if ($loadingStates[mod.title]) return;
    // Set loading immediately to prevent double-clicks
    loadingStates.update((s) => ({ ...s, [mod.title]: true }));

    // Create a closure-safe reference to the mod
    const modToInstall = { ...mod };

    // Define the actual download function
    const performDownload = async () => {
      // Re-set loading state (may be called later from dependency popup)
      loadingStates.update((s) => ({ ...s, [mod.title]: true }));
      try {
        // Create dependencies list
        const dependencies = [];
        if (modToInstall.requires_steamodded) dependencies.push("Steamodded");
        if (modToInstall.requires_talisman) dependencies.push("Talisman");

        const installedPath = await invoke<string>("install_mod", {
          url: modToInstall.downloadURL,
          folderName:
            modToInstall.folderName || modToInstall.title.replace(/\s+/g, ""),
        });

        await invoke("add_installed_mod", {
          name: modToInstall.title,
          path: installedPath,
          dependencies,
          currentVersion: modToInstall.version || "",
        });

        await getAllInstalledMods();
        installationStatus.update((s) => ({
          ...s,
          [modToInstall.title]: true,
        }));
      } catch (error) {
        console.error("Failed to install mod:", error);
        const raw = error instanceof Error ? error.message : String(error);
        const onlyUrlMsg = raw.includes("Download URL not reachable")
          ? raw.match(/Download URL not reachable[^"]*/)?.[0] || raw
          : `Failed to install ${modToInstall.title}: ${raw}`;
        addMessage(onlyUrlMsg as string, "error");
      } finally {
        loadingStates.update((s) => ({
          ...s,
          [modToInstall.title]: false,
        }));
      }
    };

    // Check dependencies first
    if (modToInstall.requires_steamodded || modToInstall.requires_talisman) {
      const steamoddedInstalled = modToInstall.requires_steamodded
        ? await invoke<boolean>("check_mod_installation", {
            modType: "Steamodded",
          })
        : true;

      const talismanInstalled = modToInstall.requires_talisman
        ? await invoke<boolean>("check_mod_installation", {
            modType: "Talisman",
          })
        : true;

      if (
        (modToInstall.requires_steamodded && !steamoddedInstalled) ||
        (modToInstall.requires_talisman && !talismanInstalled)
      ) {
        // Clear loading state before showing dependency popup
        // performDownload() will re-set it if user confirms
        loadingStates.update((s) => ({ ...s, [mod.title]: false }));
        // Key change: pass both requirements AND download function
        onCheckDependencies?.(
          {
            steamodded:
              modToInstall.requires_steamodded && !steamoddedInstalled,
            talisman: modToInstall.requires_talisman && !talismanInstalled,
          },
          performDownload,
        );
        return;
      }
    }

    // Execute download if no dependencies are missing
    await performDownload();
  };

  const isModInstalled = async (mod: Mod) => {
    if (!mod) return false;

    await getAllInstalledMods();
    const status = installedMods.some((m) => m.name === mod.title);

    // Only update the store if the status has changed
    const currentStatus = $installationStatus[mod.title];
    if (currentStatus !== status) {
      installationStatus.update((s) => ({ ...s, [mod.title]: status }));
    }

    return status;
  };

  let prevMod: Mod | null = null;

  $effect(() => {
    const newMod = $currentModView;

    // Only proceed if newMod is different from the previous mod
    if (newMod && (!prevMod || newMod.title !== prevMod.title)) {
      prevMod = newMod;
      mod = newMod;

      // Move the installation check outside of the reactive context
      setTimeout(() => {
        isModInstalled(newMod);
      }, 0);
    }
  });

  // Debounced haystack rebuild to avoid blocking main thread
  const rebuildIndex = debounce((currentMods: Mod[]) => {
    if (currentMods.length === 0) return;
    haystack = currentMods.map(
      (mod) => `${mod.title} ${mod.publisher} ${mod.description ?? ""}`,
    );
    lastModCount = currentMods.length;
  }, 100);

  onMount(() => {
    $effect(() => {
      if (searchInput) {
        searchInput.focus();
      }
    });

    // Subscribe to mods store - only rebuild index when count changes significantly
    return modsStore.subscribe((currentMods) => {
      mods = currentMods;
      // Only rebuild if mod count changed by more than 10% or first load
      const countDiff = Math.abs(currentMods.length - lastModCount);
      const threshold = Math.max(1, Math.floor(lastModCount * 0.1));
      if (
        currentMods.length > 0 &&
        (lastModCount === 0 || countDiff >= threshold)
      ) {
        rebuildIndex(currentMods);
      }
    });
  });

  const handleSearch = debounce(() => {
    if (haystack.length === 0 || searchQuery.length < 2) {
      searchResults = [];
      showSpinner = false;
      return;
    }

    isSearching = true;

    try {
      const searchTerm = searchQuery.toLowerCase();
      const results = fuzzySearch(haystack, searchTerm);

      searchResults = results.map((idx) => mods[idx]);
    } catch (error) {
      console.error("Search failed:", error);
      searchResults = [];
    } finally {
      showSpinner = false;
      isSearching = false;
    }
  }, 300);

  let showSpinner = $state(false);

  function handleInput() {
    showSpinner = true;
    handleSearch();
  }

  let scrollContainer: HTMLDivElement | null = $state(null);

  $effect(() => {
    searchResults;
    if (scrollContainer) {
      scrollContainer.scrollTop = 0;
    }
  });
</script>

<div class="search-container">
  <div class="search-bar">
    <form onsubmit={handleSearch}>
      <input
        bind:this={searchInput}
        type="text"
        bind:value={searchQuery}
        oninput={handleInput}
        placeholder="Search mods... (Author or Title)"
        class="search-input"
      />
      <!-- <button type="submit" class="search-button">
				<Search size={20} />
			</button> -->
    </form>

    {#if showSpinner}
      <!-- svelte-ignore element_invalid_self_closing_tag -->
      <div transition:fade={{ duration: 100 }} class="search-spinner" />
    {/if}
  </div>

  <div
    class="results-scroll-container default-scrollbar"
    bind:this={scrollContainer}
  >
    <div class="results-container">
      {#if isSearching}
        <p transition:fade={{ duration: 100 }} class="resulting-text">
          Searching...
        </p>
      {:else if searchResults.length === 0 && searchQuery.length >= 2}
        <p transition:fade={{ duration: 100 }} class="resulting-text">
          No mods found matching "{searchQuery}"
        </p>
      {:else if searchResults.length > 0}
        <div transition:fade={{ duration: 100 }} class="results-wrapper">
          {#each searchResults as mod (mod.downloadURL || mod.repo || mod.title)}
            <ModCard
              {mod}
              oninstallclick={installMod}
              onuninstallclick={uninstallMod}
              onmodclick={handleModClick}
              searchSpacing={true}
            />
          {/each}
        </div>
      {/if}
    </div>
  </div>
</div>

<style>
  .search-container {
    position: relative;
    /* 192px being the width of the catagories + seperator */
    width: calc(100% - 192px);
    padding: 0 1rem;
  }

  :global(::selection) {
    background: var(--ui-mod-chip-bg);
    color: var(--ui-text);
  }

  .search-bar {
    height: 3rem;
    /* accounting for the padding (2rem) & scroll container's scrollbar (0.625rem/10px)*/
    width: calc(100% - 2.625rem);
    position: absolute;
    top: 1rem;
    z-index: 100;
  }

  .search-spinner {
    display: block;
    position: absolute;
    top: 25%;
    left: calc(100% - 2.5rem);
    width: 1rem;
    height: 1rem;
    z-index: 10;
    animation: spin infinite 1s linear;
    border-radius: 9999px;
    border: 2px solid var(--ui-text);
    border-right: 2px solid transparent;
  }

  .search-bar form {
    display: flex;
    gap: 0.5rem;
    width: 100%;
  }

  .search-input {
    /* 2rem just for some spacing from the scrollbar */
    width: calc(100% - 2rem);
    padding: 0.75rem;
    border: 2px solid var(--ui-mod-input-border);
    border-radius: 6px;
    background-color: var(--ui-mod-input-bg);
    color: var(--ui-text);
    font-family: "M6X11", sans-serif;
    font-size: 1.1rem;
  }
  .search-input:focus {
    outline: none;
    border-color: var(--ui-mod-input-focus);
    transition: border-color 0.2s ease;
  }
  /* legacy search button code */
  /* .search-button {
		padding: 0.75rem 1rem;
		background: #ea9600;
		border: 2px solid #f4eee0;
		border-radius: 6px;
		color: #f4eee0;
		cursor: pointer;
		display: flex;
		align-items: center;
		transition: all 0.2s ease;
	}

	.search-button:hover {
		background: #f4eee0;
		color: #393646;
	}

	.search-button:active {
		transform: scale(0.95);
		padding: 0.75rem 0.95rem;
	} */

  .resulting-text {
    position: absolute;
  }

  .results-container {
    padding: 1rem;
    padding-top: 5rem;
    contain: layout paint;
  }

  .results-wrapper {
    width: 100%;
    height: 100%;
    display: grid;
    grid-template-columns: repeat(
      auto-fill,
      minmax(calc(300px * var(--card-scale, 1)), 1fr)
    );
    gap: 1rem;
    content-visibility: auto;
    contain-intrinsic-size: 900px 1200px;
  }

  .results-scroll-container {
    overflow-y: auto;
    height: 100%;
    contain: layout paint;
    scrollbar-gutter: stable;
    backface-visibility: hidden;
    transform: translateZ(0);
    will-change: scroll-position;
    overscroll-behavior: contain;
  }

  @media (max-width: 1160px) {
    .results-container {
      padding: 1rem;
      padding-top: 5rem;
    }
  }
</style>
