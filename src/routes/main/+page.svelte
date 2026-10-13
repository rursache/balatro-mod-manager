<script lang="ts">
  // Lazy-load ShaderBackground only when enabled
  import type { Component } from "svelte";
  let ShaderBackgroundComp = $state<Component | null>(null);
  import About from "../../components/viewblock/About.svelte";
  import LaunchButton from "../../components/LaunchButton.svelte";
  import Mods from "../../components/viewblock/Mods.svelte";
  import Settings from "../../components/viewblock/Settings.svelte";
  import RequiresPopup from "../../components/RequiresPopup.svelte";
  import SecurityPopup from "../../components/SecurityPopup.svelte";
  import LovelyMissingPopup from "../../components/LovelyMissingPopup.svelte";
  import CollectionImportPopup from "../../components/CollectionImportPopup.svelte";
  import type { DependencyCheck, InstalledMod } from "../../stores/modStore";
  import {
    currentModView,
    currentCategory,
    modsStore,
  } from "../../stores/modStore";
  import { backgroundEnabled } from "../../stores/modStore";
  import { selectedModStore, dependentsStore } from "../../stores/modStore";
  import { currentPage, paginationWindow } from "../../stores/modStore";
  import {
    installationStatus,
    showWarningPopup,
    requiresPopupStore,
    securityPopupStore,
  } from "../../stores/modStore";
  import { invoke } from "@tauri-apps/api/core";
  import { fetchCachedMods, forceRefreshCache } from "../../stores/modCache";
  import { addMessage } from "$lib/stores";
  import UninstallDialog from "../../components/UninstallDialog.svelte";
  import { onMount } from "svelte";
  import { lovelyPopupStore } from "../../stores/modStore";
  import { cardScale, darkMode } from "../../stores/ui";
  import { get } from "svelte/store";
  import ReportIssue from "../../components/ReportIssue.svelte";
  import CollectionPicker from "../../components/CollectionPicker.svelte";
  import BackupsPanel from "../../components/BackupsPanel.svelte";
  import CreateBackupModal from "../../components/CreateBackupModal.svelte";
  import RestoreBackupPopup from "../../components/RestoreBackupPopup.svelte";
  import DeleteBackupPopup from "../../components/DeleteBackupPopup.svelte";
  import { fade } from "svelte/transition";
  import { isLinuxPlatform } from "$lib/platform";
  import { backupsStore } from "../../stores/backups";
  import { clearSelection } from "../../stores/modStore";
  import {
    DEFAULT_CATALOG_CATEGORY,
    MAIN_TABS,
    categoryForTab,
    isCatalogCategory,
    isModsTab,
    resolveStartupTab,
    showsPagination,
    tabForCategory,
    type MainTab,
    type ModsTab,
  } from "../../utils/mainTabs";

  type Section = "mods" | "backups" | "settings" | "about";

  let currentSection = $state<Section>("mods");
  // Tabs stay unselected and the Mods view hidden until the installed list decides the first tab
  let startupResolved = $state(false);
  let lastCatalogCategory = $state(DEFAULT_CATALOG_CATEGORY);
  currentCategory.set(DEFAULT_CATALOG_CATEGORY);

  let activeTab = $derived<MainTab | null>(
    currentSection !== "mods"
      ? currentSection
      : startupResolved
        ? tabForCategory($currentCategory)
        : null,
  );

  $effect(() => {
    if (isCatalogCategory($currentCategory)) {
      lastCatalogCategory = $currentCategory;
    }
  });

  function openModsTab(tab: ModsTab) {
    const sameTab = tabForCategory(get(currentCategory)) === tab;
    // Coming back from Settings and the like keeps an open detail page, any other tab click closes it
    if (!(currentSection !== "mods" && sameTab)) {
      currentModView.set(null);
    }
    currentSection = "mods";
    if (!sameTab) {
      if (tab !== "installed") clearSelection();
      currentCategory.set(categoryForTab(tab, lastCatalogCategory));
    }
  }

  function selectTab(tab: MainTab) {
    startupResolved = true;
    if (isModsTab(tab)) {
      openModsTab(tab);
      return;
    }
    if (tab === "backups") backupsStore.load();
    currentSection = tab;
  }

  async function chooseStartupTab() {
    const tab = await resolveStartupTab({
      tracked: () => fetchCachedMods(),
      local: () => invoke<unknown[]>("get_detected_local_mods"),
    });
    // A tab clicked while the lists were loading wins
    if (startupResolved) return;
    currentCategory.set(categoryForTab(tab, lastCatalogCategory));
    startupResolved = true;
  }
  let isLinux = $state(false);
  let hasMounted = $state(false);
  let appVersion = $state("");

  let requiresPopupDismissedAt = 0;
  let wasRequiresPopupVisible = false;

  let storedDownloadAction: (() => Promise<void>) | null = $state(null);
  let originalDownloadAction: (() => Promise<void>) | null = $state(null);

  // Function to check if security warning needs to be shown
  async function checkSecurityAcknowledgment(): Promise<boolean> {
    try {
      const isAcknowledged = await invoke<boolean>(
        "is_security_warning_acknowledged",
      );
      return isAcknowledged;
    } catch (error) {
      console.error("Failed to check security acknowledgment:", error);
      return false; // If there's an error, show the popup anyway
    }
  }

  // Modified to include security check
  async function handleDependencyCheck(
    requirements: DependencyCheck,
    downloadAction?: () => Promise<void>,
  ) {
    if (Date.now() - requiresPopupDismissedAt < 200) {
      return;
    }
    modRequirements = requirements;
    if (downloadAction) {
      originalDownloadAction = downloadAction;

      // Check if we need to show the security popup first
      const isSecurityAcknowledged = await checkSecurityAcknowledgment();

      if (!isSecurityAcknowledged) {
        // Store the action but don't execute it yet - show security popup first
        storedDownloadAction = null;
        securityPopupStore.set({
          visible: true,
          onAcknowledge: handleSecurityAcknowledge,
          onCancel: handleSecurityCancel,
        });
      } else {
        // Security already acknowledged, proceed with dependency check
        storedDownloadAction = downloadAction;
        requiresPopupStore.set({
          visible: true,
          requiresSteamodded: requirements.steamodded,
          requiresTalisman: requirements.talisman,
          onProceed: handleProceedDownload,
          onDependencyClick: handleDependencyClick,
        });
      }
    } else {
      console.warn("handleDependencyCheck called without a download action");
      storedDownloadAction = null;
      originalDownloadAction = null;
    }
  }

  // Handle security acknowledgment
  async function handleSecurityAcknowledge() {
    securityPopupStore.update((s) => ({ ...s, visible: false }));

    // Now proceed with dependency check if there was an action
    if (originalDownloadAction) {
      storedDownloadAction = originalDownloadAction;
      requiresPopupStore.set({
        visible: true,
        requiresSteamodded: modRequirements.steamodded,
        requiresTalisman: modRequirements.talisman,
        onProceed: handleProceedDownload,
        onDependencyClick: handleDependencyClick,
      });
    }
  }

  // Handle security cancellation
  function handleSecurityCancel() {
    securityPopupStore.update((s) => ({ ...s, visible: false }));
    storedDownloadAction = null;
    originalDownloadAction = null;
  }

  function nextPage() {
    if ($currentPage < $paginationWindow.totalPages) {
      currentPage.update((n) => n + 1);
    }
  }

  function previousPage() {
    if ($currentPage > 1) {
      currentPage.update((n) => n - 1);
    }
  }

  function goToPage(page: number) {
    if (page < 1 || page > $paginationWindow.totalPages) return;
    currentPage.set(page);
  }

  function handleProceedDownload() {
    if (storedDownloadAction) {
      storedDownloadAction().catch((error) => {
        console.error("Error during download action execution:", error);
        showError(error);
      });
    } else {
      console.warn(
        "Proceed action requested, but no download action was stored.",
      );
    }
    storedDownloadAction = null; // Clear the stored action
    originalDownloadAction = null; // Clear the original action too
  }

  let contentElement: HTMLDivElement;

  let showUninstallDialog = $state(false);
  const selectedMod = $derived($selectedModStore);

  async function handleRefresh() {
    // Force-refresh cache so removal reflects immediately
    await forceRefreshCache();
    const installedMods: InstalledMod[] = await fetchCachedMods();
    installationStatus.set(
      Object.fromEntries(
        installedMods.map((mod: InstalledMod) => [mod.name, true]),
      ),
    );
  }

  function showError(error: unknown) {
    addMessage(
      `Uninstall failed: ${error instanceof Error ? error.message : String(error)}`,
      "error",
    );
  }

  function onError(event: { detail: unknown }) {
    showError(event.detail);
  }

  function onUninstalled(_event: {
    detail: { modName: string; success: boolean; action: string };
  }) {
    handleRefresh();
  }

  let modRequirements = $state({
    steamodded: false,
    talisman: false,
  });

  function handleDependencyClick(dependency: string) {
    // Find the mod in the store
    const mods = get(modsStore);
    const foundMod = mods.find(
      (m) => m.title.toLowerCase() === dependency.toLowerCase(),
    );

    // If found, open it in the mod view
    if (foundMod) {
      currentModView.set(foundMod);
    } else {
      console.warn(`Dependency mod not found: ${dependency}`);
    }
  }

  function handleRequestUninstall(
    event: CustomEvent<{ mod: InstalledMod; dependents: string[] }>,
  ) {
    selectedModStore.set(event.detail.mod);
    dependentsStore.set(event.detail.dependents);
    showUninstallDialog = true;
  }

  interface AppInitData {
    version: string;
    existing_installation: string | null;
    security_acknowledged: boolean;
    lovely_installed: boolean;
    lovely_update_available: string | null;
    launch_mode: string;
  }

  onMount(async () => {
    void chooseStartupTab();
    isLinux = await isLinuxPlatform();
    hasMounted = true;
    handleRefresh();

    // Fetch all init data in a single batched IPC call
    try {
      const initData = await invoke<AppInitData>("get_app_init_data");
      appVersion = initData.version;

      // Check if we need to show the security popup
      if (!initData.security_acknowledged) {
        securityPopupStore.set({
          visible: true,
          onAcknowledge: handleSecurityAcknowledge,
          onCancel: handleSecurityCancel,
        });
      }

      // Check Lovely status
      if (!initData.lovely_installed) {
        // Not installed: show install prompt
        lovelyPopupStore.set({ visible: true });
      } else if (initData.lovely_update_available) {
        // Lovely is installed but update available
        showWarningPopup.set({
          visible: true,
          message: `An update for Lovely (v${initData.lovely_update_available}) is available. Do you want to update?`,
          onConfirm: async () => {
            try {
              const updated = await invoke<string>("update_lovely_to_latest");
              addMessage(`Lovely updated to v${updated}`, "success");
            } catch (e) {
              addMessage(
                `Failed to update Lovely: ${e instanceof Error ? e.message : String(e)}`,
                "error",
              );
            }
            showWarningPopup.update((p) => ({ ...p, visible: false }));
          },
          onCancel: () => {
            showWarningPopup.update((p) => ({ ...p, visible: false }));
          },
        });
      }
    } catch (error) {
      console.error("Failed to load init data:", error);
      // Fallback to individual calls if batch fails
      try {
        appVersion = await invoke<string>("get_app_version");
      } catch (_) {
        appVersion = "";
      }
    }
  });

  $effect(() => {
    if (wasRequiresPopupVisible && !$requiresPopupStore.visible) {
      storedDownloadAction = null;
      originalDownloadAction = null;
      requiresPopupDismissedAt = Date.now();
    }
    wasRequiresPopupVisible = $requiresPopupStore.visible;
  });

  $effect(() => {
    if (!hasMounted || isLinux) {
      return;
    }

    if ($backgroundEnabled && !ShaderBackgroundComp) {
      import("../../components/ShaderBackground.svelte")
        .then((m) => {
          ShaderBackgroundComp = m.default;
        })
        .catch(() => {});
    }
  });
</script>

<!-- Background shader is dynamically imported below when enabled -->

{#if $backgroundEnabled && ShaderBackgroundComp && !isLinux}
  <ShaderBackgroundComp darkMode={$darkMode} />
{/if}

<div class="main-page">
  <header class="app-header" data-tauri-drag-region>
    <div class="brand" data-tauri-drag-region>
      <h1 data-tauri-drag-region>Balatro Mod Manager</h1>
      {#if appVersion}
        <span class="app-version" data-testid="header-version"
          >v{appVersion}</span
        >
      {/if}
    </div>
    <nav class="tabs" aria-label="Sections">
      {#each MAIN_TABS as tab (tab.id)}
        <button
          class:active={activeTab === tab.id}
          aria-current={activeTab === tab.id ? "page" : undefined}
          onclick={() => selectTab(tab.id)}
        >
          {tab.label}
        </button>
      {/each}
    </nav>
    <div class="launch-slot">
      <LaunchButton />
    </div>
  </header>

  <div
    class="content"
    class:modal-open={!!$currentModView && currentSection == "mods"}
    bind:this={contentElement}
    style="--card-scale: {$cardScale}"
  >
    <!-- All sections stay mounted for smooth transitions -->
    <div
      class="section-wrapper"
      class:active={currentSection === "mods" && startupResolved}
    >
      <Mods mod={null} {handleDependencyCheck} />
    </div>

    <div class="section-wrapper" class:active={currentSection === "backups"}>
      <BackupsPanel />
    </div>

    <div class="section-wrapper" class:active={currentSection === "settings"}>
      <Settings />
    </div>

    <div class="section-wrapper" class:active={currentSection === "about"}>
      <About version={appVersion} />
    </div>
  </div>

  {#if currentSection === "mods" && startupResolved && showsPagination($currentCategory)}
    <div class="pagination-footer">
      {#if !$currentModView && $paginationWindow.totalPages > 1}
        <div
          class="pagination-controls"
          in:fade={{ duration: 150 }}
          out:fade={{ duration: 120 }}
        >
          <button onclick={previousPage} disabled={$currentPage === 1}>
            Previous
          </button>
          {#each Array(Math.min($paginationWindow.maxVisiblePages, $paginationWindow.totalPages)) as _, i (i)}
            {#if $paginationWindow.startPage + i <= $paginationWindow.totalPages}
              <button
                class:active={$currentPage === $paginationWindow.startPage + i}
                onclick={() => goToPage($paginationWindow.startPage + i)}
              >
                {$paginationWindow.startPage + i}
              </button>
            {/if}
          {/each}
          <button
            onclick={nextPage}
            disabled={$currentPage === $paginationWindow.totalPages}
          >
            Next
          </button>
        </div>
      {/if}
    </div>
  {/if}

  <RequiresPopup />

  <SecurityPopup />

  <UninstallDialog
    bind:show={showUninstallDialog}
    modName={selectedMod?.name ?? ""}
    modPath={selectedMod?.path ?? ""}
    bind:dependents={$dependentsStore}
    {onUninstalled}
    {onError}
  />
</div>

<LovelyMissingPopup />
<CollectionPicker />
<CollectionImportPopup />
<ReportIssue />
<CreateBackupModal />
<RestoreBackupPopup />
<DeleteBackupPopup />

<style>
  .main-page {
    width: 100vw;
    height: 100vh;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 0.75rem 1.5rem 1rem;
    box-sizing: border-box;
    background: transparent;
  }

  .app-header {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    grid-template-areas:
      "brand launch"
      "tabs launch";
    align-items: center;
    column-gap: 1rem;
    row-gap: 0.6rem;
  }

  .brand {
    grid-area: brand;
    display: flex;
    align-items: baseline;
    gap: 0.6rem;
    min-width: 0;
  }

  h1 {
    margin: 0;
    color: var(--ui-text);
    font-size: 2.4rem;
    line-height: 1;
    font-family: "M6X11", sans-serif;
    white-space: nowrap;
    text-shadow:
      -2px -2px 0 #000,
      2px -2px 0 #000,
      -2px 2px 0 #000,
      2px 2px 0 #000;
  }

  .app-version {
    color: var(--ui-text);
    opacity: 0.65;
    font-family: "M6X11", sans-serif;
    font-size: 1rem;
    white-space: nowrap;
    user-select: none;
  }

  .launch-slot {
    grid-area: launch;
    align-self: center;
  }

  .tabs {
    grid-area: tabs;
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    min-width: 0;
  }

  .tabs button {
    background: transparent;
    border: 2px solid var(--ui-text);
    color: var(--ui-text);
    padding: 0.5rem 1rem;
    border-radius: 8px;
    font-family: "M6X11", sans-serif;
    font-size: 1.1rem;
    white-space: nowrap;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .tabs button:hover,
  .tabs button.active {
    background: var(--ui-mod-chip-active-bg);
    color: var(--ui-mod-chip-active-text);
  }

  .content {
    position: relative;
    flex: 1;
    min-height: 0;
    background: var(--ui-danger-overlay);
    border-radius: 5px;
    backdrop-filter: blur(10px);
    outline: 2px solid var(--ui-danger-overlay-border-strong);
    overflow: hidden;
  }

  .content.modal-open {
    overflow: hidden !important;
    padding-right: 0;
  }

  :global(:root) {
    --scrollbar-width: 10px;
  }

  .section-wrapper {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    opacity: 0;
    visibility: hidden;
    transition:
      opacity 0.2s ease,
      visibility 0.2s ease;
    overflow-y: auto;
    overflow-x: hidden;
  }

  .section-wrapper::-webkit-scrollbar {
    width: 10px;
  }

  .section-wrapper::-webkit-scrollbar-track {
    background: transparent;
    border-radius: 15px;
  }

  .section-wrapper::-webkit-scrollbar-thumb {
    background: var(--ui-scroll-thumb);
    border: 2px solid var(--ui-scroll-thumb-border);
    border-radius: 15px;
  }

  .section-wrapper.active {
    opacity: 1;
    visibility: visible;
  }

  /* Reserved for every paged view so the grid does not jump when the controls fade */
  .pagination-footer {
    height: 2.1rem;
    margin-top: -0.25rem;
    display: flex;
    justify-content: center;
    align-items: center;
    flex-shrink: 0;
  }

  .pagination-controls {
    display: flex;
    gap: 0.4rem;
  }

  .pagination-controls button {
    padding: 0.45rem 0.9rem;
    background: var(--ui-mod-chip-bg);
    border: 1px solid var(--ui-mod-chip-border);
    color: var(--ui-mod-chip-text);
    font-family: "M6X11", sans-serif;
    font-size: 0.9rem;
    cursor: pointer;
    border-radius: 3px;
    transition: all 0.2s ease;
  }

  .pagination-controls button:hover:not(:disabled) {
    background: var(--ui-mod-chip-active-bg);
    color: var(--ui-mod-chip-active-text);
  }

  .pagination-controls button.active {
    background: var(--ui-mod-chip-active-bg);
    color: var(--ui-mod-chip-active-text);
  }

  .pagination-controls button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  :global([data-platform="linux"]) .content {
    backdrop-filter: none;
    background: var(--ui-danger-overlay-strong);
  }

  @media (max-width: 1160px) {
    .main-page {
      padding: 0.6rem 1.1rem 0.9rem;
      gap: 0.6rem;
    }

    h1 {
      font-size: 2.1rem;
    }

    .tabs {
      gap: 0.4rem;
    }

    .tabs button {
      padding: 0.45rem 0.75rem;
      font-size: 0.95rem;
    }
  }
</style>
