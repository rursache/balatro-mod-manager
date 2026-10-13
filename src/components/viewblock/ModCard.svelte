<script lang="ts">
  import type { Mod } from "../../stores/modStore";
  import {
    Download,
    Trash2,
    RefreshCw,
    Layers,
    Check,
    User,
    ArrowDownToLine,
  } from "lucide-svelte";
  import {
    installationStatus,
    loadingStates2 as loadingStates,
    modEnabledStore,
    updateAvailableStore,
    selectedModsStore,
    toggleModSelection,
  } from "../../stores/modStore";
  import { descriptionsStore } from "../../stores/descriptions";
  import { openCollectionPicker } from "../../stores/collections";
  import { stripMarkdown } from "../../utils/helpers";
  import { invoke } from "@tauri-apps/api/core";
  import { lovelyPopupStore } from "../../stores/modStore";
  import { forceRefreshCache } from "../../stores/modCache";
  import LazyImage from "../common/LazyImage.svelte";
  import LastUpdated from "./LastUpdated.svelte";
  import { cardScale, darkMode } from "../../stores/ui";
  import { normalizeColorPair, pickDarkPalette } from "../../utils/cardPalette";
  import { observeResize } from "../../utils/resize-observer-pool";
  import { rowPreloadMargin } from "../../utils/thumbnailRequests";
  import { onMount, onDestroy } from "svelte";
  import { isLinuxPlatform } from "$lib/platform";
  import { formatDownloads } from "../../utils/catalogFilters";

  interface Props {
    mod: Mod;
    onmodclick?: (mod: Mod) => void;
    oninstallclick?: (mod: Mod) => void;
    onuninstallclick?: (mod: Mod) => void;
    onToggleEnabled?: () => Promise<void>;
    deferImages?: boolean;
    searchSpacing?: boolean;
    hideDescription?: boolean;
    disableInstall?: boolean;
    showCheckbox?: boolean;
  }

  let {
    mod,
    oninstallclick,
    onuninstallclick,
    onmodclick,
    onToggleEnabled,
    deferImages = false,
    searchSpacing = false,
    hideDescription = false,
    disableInstall = false,
    showCheckbox = false,
  }: Props = $props();

  let isEnabled = $state(true); // Default to enabled if not yet checked
  let isLinux = false;
  let isSelected = $derived($selectedModsStore.has(mod.title));
  let descriptionText = $derived(
    $descriptionsStore[mod.title] ?? mod.description ?? "",
  );
  let thumbLoaded = $state(false);
  let lastThumbKey = "";
  let titleEl: HTMLHeadingElement | null = $state(null);
  let titleScale = $state(1);
  let unobserveTitle: (() => void) | null = null;
  let downloadsLabel = $derived(formatDownloads(mod.downloads));
  let sourceBadges = $derived(
    mod.source === "merged"
      ? ["index", "thunderstore"]
      : mod.source === "thunderstore"
        ? ["thunderstore"]
        : [],
  );
  let cardColors = $derived(
    $darkMode ? pickDarkPalette(mod.title) : normalizeColorPair(mod.colors),
  );

  $effect(() => {
    const key = `${mod.title}|${mod.image}`;
    if (key !== lastThumbKey) {
      lastThumbKey = key;
      thumbLoaded = false;
    }
  });

  function handleThumbDone() {
    thumbLoaded = true;
  }

  // Sync local isEnabled state with the store
  $effect(() => {
    const storeValue = $modEnabledStore[mod.title];
    if (storeValue !== undefined) {
      isEnabled = storeValue;
    }
  });

  onMount(async () => {
    isLinux = await isLinuxPlatform();
  });

  const updateTitleScale = () => {
    if (!titleEl || !hideDescription) {
      titleScale = 1;
      return;
    }
    const available = titleEl.clientWidth;
    const needed = titleEl.scrollWidth;
    if (available > 0 && needed > 0) {
      const ratio = Math.min(1, available / needed);
      titleScale = Math.max(0.8, ratio);
    } else {
      titleScale = 1;
    }
  };

  // Debounced version of updateTitleScale for ResizeObserver
  let resizeDebounceTimer: ReturnType<typeof setTimeout> | null = null;
  const debouncedUpdateTitleScale = () => {
    if (resizeDebounceTimer) clearTimeout(resizeDebounceTimer);
    resizeDebounceTimer = setTimeout(updateTitleScale, 50);
  };

  $effect(() => {
    if (!hideDescription) {
      titleScale = 1;
      return;
    }
    mod.title;
    $cardScale;
    requestAnimationFrame(updateTitleScale);
  });

  onMount(() => {
    if (titleEl) {
      unobserveTitle = observeResize(titleEl, debouncedUpdateTitleScale);
    }
  });

  onDestroy(() => {
    if (resizeDebounceTimer) clearTimeout(resizeDebounceTimer);
    unobserveTitle?.();
    unobserveTitle = null;
  });

  async function toggleModEnabled(e: Event) {
    e.stopPropagation();
    try {
      const currentState = $modEnabledStore[mod.title] ?? isEnabled;
      const newState = !currentState;

      await invoke("toggle_mod_enabled", {
        modName: mod.title,
        enabled: newState,
      });

      // Update both the store and local variable
      modEnabledStore.update((enabledMods) => ({
        ...enabledMods,
        [mod.title]: newState,
      }));
      isEnabled = newState;

      // Call the parent callback to update the filtered lists
      if (onToggleEnabled) {
        await onToggleEnabled();
      }
    } catch (error) {
      console.error(`Failed to toggle mod ${mod.title} enabled state:`, error);
    }
  }
  function installMod(e: Event) {
    e.stopPropagation();
    // Guard: don't allow re-entrancy or duplicate installs
    if (
      $loadingStates[mod.title] ||
      $installationStatus[mod.title] ||
      (disableInstall && !$installationStatus[mod.title])
    ) {
      return;
    }
    if (mod.title.toLowerCase() === "steamodded") {
      // Set loading immediately to prevent multiple clicks while fetching URL
      loadingStates.update((s) => ({ ...s, [mod.title]: true }));
      fetchAndInstallLatestSteamodded().catch(() => {
        // ensure loading is cleared on early failure
        loadingStates.update((s) => ({ ...s, [mod.title]: false }));
      });
    } else if (oninstallclick) {
      oninstallclick(mod);
    }
  }

  function updateMod(e: Event) {
    e.stopPropagation();
    // Guard: don't allow re-entrancy while loading
    if ($loadingStates[mod.title]) {
      return;
    }
    // Reuse the install logic but for updating
    if (mod.title.toLowerCase() === "steamodded") {
      loadingStates.update((s) => ({ ...s, [mod.title]: true }));
      fetchAndInstallLatestSteamodded().catch(() => {
        loadingStates.update((s) => ({ ...s, [mod.title]: false }));
      });
    } else if (oninstallclick) {
      oninstallclick(mod);
    }
  }

  function uninstallMod(e: Event) {
    e.stopPropagation();
    if (onuninstallclick) onuninstallclick(mod);
  }

  function openCollections(e: Event) {
    e.stopPropagation();
    openCollectionPicker(mod.title, mod.id);
  }

  function openModView() {
    if (onmodclick) onmodclick(mod);
  }

  function handleCheckboxClick(e: Event) {
    e.stopPropagation();
    toggleModSelection(mod.title);
  }

  async function fetchAndInstallLatestSteamodded() {
    try {
      const latestReleaseURL = await invoke<string>(
        "get_latest_steamodded_release",
      );
      await installModFromURL(latestReleaseURL);
    } catch (error) {
      console.error("Failed to get latest Steamodded release:", error);
      throw error;
    }
  }

  async function installModFromURL(url: string, folder_name: string = "") {
    const wasInstalled = Boolean($installationStatus[mod.title]);
    let desiredEnabledState = true;

    if (wasInstalled) {
      let previousEnabled = $modEnabledStore[mod.title];
      if (previousEnabled === undefined) {
        try {
          previousEnabled = await invoke<boolean>("is_mod_enabled", {
            modName: mod.title,
          });
        } catch (error) {
          console.error(
            `Failed to read existing enabled state for ${mod.title}:`,
            error,
          );
        }
      }

      if (previousEnabled !== undefined) {
        desiredEnabledState = previousEnabled;
      } else {
        desiredEnabledState = isEnabled;
      }
    }

    try {
      loadingStates.update((s) => ({ ...s, [mod.title]: true }));

      // Show a warning if Lovely injector is missing (do not block installation)
      if (!isLinux) {
        try {
          const present = await invoke<boolean>("is_lovely_installed");
          if (!present) {
            lovelyPopupStore.set({ visible: true });
          }
        } catch (_) {
          /* ignore */
        }
      }

      if (!/^(https?|index|bmi):\/\//i.test(url)) {
        console.error("Invalid URL format:", url);
        throw new Error(`Invalid URL format: ${url}`);
      }

      // Use mod title as fallback if folder_name is empty
      const folderName = folder_name || mod.title || "";

      const installedPath = await invoke<string>("install_mod", {
        url,
        folderName,
      });

      await invoke("add_installed_mod", {
        name: mod.title,
        path: installedPath,
        dependencies: mod.requires_steamodded ? ["Steamodded"] : [],
        currentVersion: mod.version || "",
      });

      installationStatus.update((s) => ({ ...s, [mod.title]: true }));

      // After installing/updating, reset update status
      updateAvailableStore.update((updates) => ({
        ...updates,
        [mod.title]: false,
      }));

      modEnabledStore.update((enabledMods) => ({
        ...enabledMods,
        [mod.title]: desiredEnabledState,
      }));
      isEnabled = desiredEnabledState;

      // After install, verify Lovely is still present
      if (!isLinux) {
        try {
          const present = await invoke<boolean>("is_lovely_installed");
          if (!present) {
            lovelyPopupStore.set({ visible: true });
          }
        } catch (_) {
          /* ignore */
        }
      }
    } catch (error) {
      console.error("Failed to install mod:", error);
    } finally {
      loadingStates.update((s) => ({ ...s, [mod.title]: false }));
      // Keep cache in sync so other views reflect installation immediately
      try {
        await forceRefreshCache();
      } catch (_) {
        /* ignore */
      }
    }
  }
  // Truncate description based on current card scale; this avoids overflow
  // even when CSS multi-line clamp support is inconsistent across platforms.
  function truncateDynamic(text: string, scale: number): string {
    if (!text) return "";
    const lines = scale <= 0.85 ? 1 : 2;
    // width and font-size scale together → per-line capacity ~ constant
    const basePerLine = 38;
    const maxChars = Math.max(18, basePerLine * lines);
    return text.length > maxChars
      ? text.slice(0, maxChars).trimEnd() + "..."
      : text;
  }
</script>

<div
  class="mod-card"
  class:compact={$cardScale <= 0.85}
  class:desc-hidden={hideDescription}
  class:thumb-loading={!thumbLoaded}
  class:search-spacing={searchSpacing}
  class:selected={isSelected}
  onclick={openModView}
  onkeydown={(e) => e.key === "Enter" && openModView()}
  role="button"
  tabindex="0"
  style="--orig-color1: {cardColors.color1}; --orig-color2: {cardColors.color2};"
>
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
  <div class="mod-image">
    <LazyImage
      src={mod.image}
      fallbackSrc={mod.imageFallback || "/images/cover.jpg"}
      alt={mod.title}
      cacheTitle={mod.title}
      cacheId={mod._dirName}
      remoteSrc={mod.thumbnailUrl}
      rootMargin={rowPreloadMargin($cardScale)}
      deferLoad={deferImages}
      hasThumbnail={mod._hasThumbnail !== false}
      on:load={handleThumbDone}
      on:error={handleThumbDone}
    />

    {#if sourceBadges.length > 0 || mod.deprecated || mod.aiGenerated || mod.nsfw}
      <div class="badges" data-testid="mod-badges">
        {#each sourceBadges as source (source)}
          <span
            class="badge source {source}"
            title={source === "thunderstore"
              ? "Listed on Thunderstore"
              : "Listed in the mod index"}
          >
            {source === "thunderstore" ? "Thunderstore" : "Index"}
          </span>
        {/each}
        {#if mod.deprecated}
          <span
            class="badge deprecated"
            title="Deprecated by its author on Thunderstore"
          >
            Deprecated
          </span>
        {/if}
        {#if mod.aiGenerated}
          <span class="badge ai" title="Tagged AI Generated on Thunderstore">
            AI
          </span>
        {/if}
        {#if mod.nsfw}
          <span class="badge nsfw" title="Marked NSFW on Thunderstore">
            NSFW
          </span>
        {/if}
      </div>
    {/if}

    {#if !mod.orphaned && (mod.publisher || downloadsLabel)}
      <div class="meta-tags">
        {#if mod.publisher}
          <span class="tag meta-owner" title={`By ${mod.publisher}`}>
            <User size={11} />
            <span class="meta-text">{mod.publisher}</span>
          </span>
        {/if}
        {#if downloadsLabel}
          <span
            class="tag meta-downloads"
            title={`${mod.downloads?.toLocaleString()} downloads on Thunderstore`}
          >
            <ArrowDownToLine size={11} />
            {downloadsLabel}
          </span>
        {/if}
      </div>
    {/if}

    <div class="tags">
      {#if mod.orphaned}
        <span
          class="tag orphaned"
          title="This mod was removed from the remote index. It is still installed locally and can be uninstalled at any time."
        >
          Removed from index
        </span>
        {#if mod.installedVersion}
          <span class="tag version" title="Installed version">
            {mod.installedVersion}
          </span>
        {/if}
      {/if}
      <LastUpdated value={mod.last_updated} class="tag updated" iconSize={11} />
    </div>
  </div>

  <div class="mod-info">
    <h3 bind:this={titleEl} style={`--title-scale: ${titleScale}`}>
      {mod.title}
    </h3>
    {#if !hideDescription && descriptionText && descriptionText.trim().length > 0}
      <p class="fade-in">
        {truncateDynamic(stripMarkdown(descriptionText), $cardScale)}
      </p>
    {:else if !hideDescription}
      <div class="desc-skeleton" aria-hidden="true">
        <div class="line" style="width: 92%"></div>
        <div class="line" style="width: 84%"></div>
        <div class="line" style="width: 68%"></div>
      </div>
    {/if}
  </div>

  <div class="button-container">
    {#if $installationStatus[mod.title]}
      <!-- Enable/Disable button (only shown when mod is installed) -->
      <button
        class="toggle-button"
        class:enabled={$modEnabledStore[mod.title] ?? isEnabled}
        class:disabled={!($modEnabledStore[mod.title] ?? isEnabled)}
        title={($modEnabledStore[mod.title] ?? isEnabled)
          ? "Disable Mod"
          : "Enable Mod"}
        onclick={toggleModEnabled}
      >
        {#if $modEnabledStore[mod.title] ?? isEnabled}
          ON
        {:else}
          OFF
        {/if}
      </button>
    {/if}

    {#if $installationStatus[mod.title] && $updateAvailableStore[mod.title] && !mod.orphaned}
      <!-- Update button (when installed and update available) -->
      <button
        class="update-button"
        onclick={updateMod}
        disabled={$loadingStates[mod.title]}
      >
        {#if $loadingStates[mod.title]}
          <div class="spinner"></div>
        {:else}
          <RefreshCw size={18} />
          Update
        {/if}
      </button>
    {:else}
      <!-- Regular download/installed button -->
      <button
        class="download-button"
        class:installed={$installationStatus[mod.title]}
        disabled={$installationStatus[mod.title] ||
          $loadingStates[mod.title] ||
          (disableInstall && !$installationStatus[mod.title])}
        onclick={installMod}
      >
        {#if $loadingStates[mod.title]}
          <div class="spinner"></div>
        {:else}
          <Download size={18} />
          {$installationStatus[mod.title] ? "Installed" : "Download"}
        {/if}
      </button>
    {/if}

    {#if $installationStatus[mod.title]}
      <button class="delete-button" title="Remove Mod" onclick={uninstallMod}>
        <Trash2 size={18} />
      </button>
    {/if}

    {#if !mod.orphaned}
      <button
        class="collection-button"
        title="Add to collection"
        onclick={openCollections}
      >
        <Layers size={18} />
      </button>
    {/if}
  </div>
</div>

<style>
  .mod-card {
    --bg-color: var(--orig-color1, #4f6367);
    --bg-color-2: var(--orig-color2, #334461);

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
    cursor: pointer;
    background-size: 100% 200%;
    transition: all 0.3s ease;
    background-image: repeating-linear-gradient(
      -45deg,
      var(--bg-color),
      var(--bg-color) 10px,
      var(--bg-color-2) 10px,
      var(--bg-color-2) 20px
    );
    will-change: transform;
    backface-visibility: hidden;
  }

  .mod-card.search-spacing {
    margin: 1rem auto 0.5rem;
  }

  .mod-card.thumb-loading {
    content-visibility: visible;
  }

  .mod-card:hover {
    animation: stripe-slide-up 2.5s linear infinite;
    scale: 1.05;
  }

  :global([data-platform="linux"]) .mod-card:hover {
    animation: none;
  }

  /* Disable animation for users who prefer reduced motion */
  @media (prefers-reduced-motion: reduce) {
    .mod-card:hover {
      animation: none;
    }
  }

  @keyframes stripe-slide-up {
    0% {
      background-position: 0 0;
    }
    100% {
      background-position: 0 -55px;
    }
  }

  .mod-image {
    position: relative;
    height: calc(150px * var(--card-scale, 1));
  }

  /* Image styling handled inside LazyImage */

  .tags {
    position: absolute;
    top: 7.2rem;
    right: 0.35rem;
    display: flex;
    gap: 0.5rem;
  }

  .tags :global(.tag) {
    font-family: "M6X11", monospace;
    font-size: calc(0.75rem * var(--card-scale, 1));
    padding: 0.2rem 0.45rem;
    border-radius: 4px;
    line-height: 1;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    backdrop-filter: blur(2px);
  }

  .badges {
    position: absolute;
    top: 0.35rem;
    right: 0.35rem;
    left: 2.4rem;
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 0.3rem;
    pointer-events: none;
  }

  .badge {
    font-family: "M6X11", monospace;
    font-size: calc(0.72rem * var(--card-scale, 1));
    padding: 0.2rem 0.4rem;
    border-radius: 4px;
    line-height: 1;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: #fff;
    border: 1px solid rgba(255, 255, 255, 0.25);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
    pointer-events: auto;
  }

  .badge.source.index {
    background: rgba(20, 20, 20, 0.72);
  }

  .badge.source.thunderstore {
    background: rgba(35, 87, 140, 0.92);
  }

  .badge.deprecated {
    background: rgba(192, 64, 56, 0.92);
  }

  .badge.ai {
    background: rgba(121, 110, 158, 0.95);
  }

  .badge.nsfw {
    background: rgba(160, 40, 90, 0.95);
  }

  .meta-tags {
    position: absolute;
    top: 7.2rem;
    left: 0.35rem;
    display: flex;
    gap: 0.3rem;
    max-width: 55%;
    min-width: 0;
  }

  .meta-tags .tag {
    font-family: "M6X11", monospace;
    font-size: calc(0.75rem * var(--card-scale, 1));
    padding: 0.2rem 0.45rem;
    border-radius: 4px;
    line-height: 1;
    letter-spacing: 0.02em;
    backdrop-filter: blur(2px);
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    background: rgba(20, 20, 20, 0.72);
    color: #fff;
    border: 1px solid rgba(255, 255, 255, 0.2);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
    text-transform: none;
    min-width: 0;
  }

  .meta-owner .meta-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta-downloads {
    flex-shrink: 0;
  }

  .tag.orphaned {
    background: rgba(192, 64, 56, 0.92);
    color: #fff;
    border: 1px solid rgba(255, 255, 255, 0.25);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
  }

  .tags .tag.version {
    background: rgba(20, 20, 20, 0.72);
    color: #fff;
    border: 1px solid rgba(255, 255, 255, 0.2);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
    text-transform: none;
    max-width: 7rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tags :global(.tag.updated) {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    background: rgba(20, 20, 20, 0.72);
    color: #fff;
    border: 1px solid rgba(255, 255, 255, 0.2);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
  }

  .mod-info {
    flex: 1;
    padding: 0.5rem;
    position: relative;
    bottom: 1rem;
    /* Reserve space for buttons at bottom to prevent content overflow */
    padding-bottom: calc(50px * var(--card-scale, 1));
    overflow: hidden;
  }

  .mod-info > p {
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow: hidden;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    padding: 0 0.1rem;
    word-break: break-word;
    overflow-wrap: anywhere;
    text-overflow: ellipsis;
  }

  .mod-info > p.fade-in {
    animation: fade-in 0.3s ease-out;
  }

  @keyframes fade-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  .mod-info h3 {
    color: var(--ui-heading);
    font-size: calc(1.5rem * var(--card-scale, 1));
    margin-bottom: 0.2rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .mod-card.desc-hidden .mod-info h3 {
    font-size: calc(2rem * var(--card-scale, 1) * var(--title-scale, 1));
    margin-bottom: 0.4rem;
    margin-top: 0.4rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .mod-info p {
    color: var(--ui-text);
    font-size: calc(1rem * var(--card-scale, 1));
    line-height: 1.2;
  }

  /* Tighten description to 1 line for compact cards */
  .mod-card.compact .mod-info > p {
    -webkit-line-clamp: 1;
    line-clamp: 1;
  }

  .mod-card.desc-hidden {
    height: calc(275px * var(--card-scale, 1));
  }

  .mod-card.desc-hidden .mod-image {
    height: calc(130px * var(--card-scale, 1));
  }

  .mod-card.desc-hidden .mod-info {
    bottom: -0.4rem;
    padding-bottom: calc(44px * var(--card-scale, 1));
    padding-top: 0.2rem;
  }

  .mod-card.desc-hidden .mod-info h3 {
    margin-top: 0.4rem;
  }

  .mod-card.desc-hidden .button-container {
    bottom: 0.4rem;
  }

  /* Description skeleton */
  .desc-skeleton {
    margin-top: 0.2rem;
  }
  .desc-skeleton .line {
    height: 12px;
    margin: 6px 0;
    border-radius: 6px;
    background: linear-gradient(
      90deg,
      var(--ui-glass-weak) 25%,
      var(--ui-glass-strong) 37%,
      var(--ui-glass-weak) 63%
    );
    background-size: 400% 100%;
    animation: shimmer 1.2s ease-in-out infinite;
  }

  @keyframes shimmer {
    0% {
      background-position: 100% 0;
    }
    100% {
      background-position: 0 0;
    }
  }

  .button-container {
    display: flex;
    gap: 0.35rem;
    position: absolute;
    bottom: 1rem;
    left: 1rem;
    width: calc(100% - 2rem);
  }

  .download-button,
  .update-button {
    flex: 1;
    min-width: 0; /* Allow shrinking */
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.4rem;
    padding: calc(0.6rem * var(--card-scale, 1))
      calc(0.5rem * var(--card-scale, 1));
    border: none;
    border-radius: 4px;
    font-family: "M6X11", sans-serif;
    font-size: calc(0.95rem * var(--card-scale, 1));
    cursor: pointer;
    transition: all 0.2s ease;
    min-height: calc(38px * var(--card-scale, 1));
    position: relative;
  }

  .download-button {
    background: var(--ui-success);
    color: var(--ui-text);
    outline: var(--ui-button-green-border) solid 2px;
  }

  .update-button {
    background: var(--ui-info);
    color: var(--ui-text);
    outline: var(--ui-info-strong) solid 2px;
  }

  .update-button:hover {
    background: var(--ui-info-hover); /* Lighter blue on hover */
    transform: translateY(-2px);
  }

  .update-button:active {
    transform: translateY(1px);
  }

  .download-button:hover:not(.installed) {
    background: var(--ui-success-hover);
    transform: translateY(-2px);
  }

  .download-button.installed {
    background: var(--ui-neutral);
    outline-color: var(--ui-neutral-outline);
    cursor: not-allowed;
  }

  .download-button:active:not(.installed) {
    transform: translateY(1px);
  }

  /* Compact adjustments: make primary buttons a touch smaller */
  .mod-card.compact .download-button,
  .mod-card.compact .update-button {
    padding: calc(0.5rem * var(--card-scale, 1))
      calc(0.4rem * var(--card-scale, 1));
    min-height: calc(34px * var(--card-scale, 1));
    font-size: calc(0.85rem * var(--card-scale, 1));
  }

  .mod-card.compact .toggle-button,
  .mod-card.compact .delete-button,
  .mod-card.compact .collection-button {
    min-width: calc(32px * var(--card-scale, 1));
    height: calc(34px * var(--card-scale, 1));
    padding: calc(4px * var(--card-scale, 1));
  }

  .mod-card.compact .toggle-button {
    min-width: calc(38px * var(--card-scale, 1));
    font-size: calc(0.9rem * var(--card-scale, 1));
  }

  .mod-card.compact .button-container {
    bottom: 0.7rem;
  }

  .delete-button {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: calc(36px * var(--card-scale, 1));
    height: calc(38px * var(--card-scale, 1));
    padding: calc(6px * var(--card-scale, 1));
    background: var(--ui-danger);
    color: var(--ui-text);
    border: none;
    outline: var(--ui-danger-outline) solid 2px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.2s ease;
    font-family: "M6X11", sans-serif;
    font-size: calc(1rem * var(--card-scale, 1));
    flex-shrink: 0;
  }

  .delete-button:hover {
    background: var(--ui-danger-hover);
    transform: translateY(-2px);
  }

  .delete-button:active {
    transform: translateY(1px);
  }

  .collection-button {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: calc(36px * var(--card-scale, 1));
    height: calc(38px * var(--card-scale, 1));
    padding: calc(6px * var(--card-scale, 1));
    background: var(--ui-info-strong);
    color: var(--ui-text);
    border: none;
    outline: var(--ui-info-outline) solid 2px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.2s ease;
    flex-shrink: 0;
  }

  .collection-button:hover {
    background: var(--ui-info-strong-hover);
    transform: translateY(-2px);
  }

  .collection-button:active {
    transform: translateY(1px);
  }

  /* Enable/Disable toggle button styles */
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
    color: white;
    border: none;
    flex-shrink: 0;
    font-family: "M6X11", sans-serif;
    font-size: calc(1rem * var(--card-scale, 1));
  }

  .toggle-button.enabled {
    background: var(--ui-success-strong); /* Bright green when enabled */
    outline: #1a7a42 solid 2px; /* Darker green border for visibility */
  }

  .toggle-button.disabled {
    background: var(--ui-neutral); /* Gray when disabled, instead of red */
    outline: var(--ui-neutral-outline) solid 2px;
  }

  .toggle-button:hover.enabled {
    background: var(--ui-success-strong-hover); /* Lighter green on hover */
    outline: #1a7a42 solid 2px; /* Keep darker border on hover */
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

  .download-button:disabled,
  .update-button:disabled {
    opacity: 0.8;
    cursor: not-allowed;
  }

  @media (max-width: 1160px) {
    .mod-card {
      width: 100%;
    }
  }

  .spinner {
    border: 2px solid var(--ui-glass-border);
    border-top: 2px solid var(--ui-text);
    border-radius: 50%;
    width: calc(16px * var(--card-scale, 1));
    height: calc(16px * var(--card-scale, 1));
    animation: spin 1s linear infinite;
    /* Center the spinner while maintaining button size */
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

  /* Selection checkbox */
  .select-checkbox {
    position: absolute;
    top: 0.5rem;
    left: 0.5rem;
    width: 24px;
    height: 24px;
    border-radius: 4px;
    border: 2px solid white;
    background: rgba(0, 0, 0, 0.5);
    cursor: pointer;
    z-index: 10;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s ease;
    padding: 0;
  }

  .select-checkbox:hover {
    border-color: white;
    background: rgba(0, 0, 0, 0.7);
  }

  .select-checkbox.checked {
    background: var(--ui-success);
    border-color: white;
  }

  .mod-card.selected {
    outline: 2px solid var(--ui-success);
    outline-offset: -2px;
  }
</style>
