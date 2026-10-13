<script lang="ts">
  import { fade } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import {
    Download,
    Trash2,
    User,
    ArrowLeft,
    RefreshCw,
    Layers,
    ExternalLink,
    ArrowDownToLine,
  } from "lucide-svelte";
  import Github from "../GithubIcon.svelte";
  import LastUpdated from "./LastUpdated.svelte";
  import { onMount, onDestroy } from "svelte";
  import {
    currentModView,
    installationStatus,
    loadingStates2 as loadingStates,
    uninstallDialogStore,
    currentCategory,
    updateAvailableStore,
    currentPage,
    modEnabledStore,
  } from "../../stores/modStore";
  import type { InstalledMod, Mod } from "../../stores/modStore";
  import { marked } from "marked";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { cachedVersions } from "../../stores/modStore";
  import { addMessage } from "$lib/stores";
  import { lovelyPopupStore } from "../../stores/modStore";
  import { modsStore } from "../../stores/modStore";
  import { untrack } from "svelte";
  import { fetchCachedMods, forceRefreshCache } from "../../stores/modCache";
  import { descriptionsStore, setDescription } from "../../stores/descriptions";
  import LazyImage from "../common/LazyImage.svelte";
  import { isLinuxPlatform } from "$lib/platform";
  import { isAnyPopupOpen } from "../../stores/popupManager";
  import { openExternal } from "$lib/opener";
  import {
    openCollectionPicker,
    collectionsStore,
    activeCollectionIds,
    removeActiveCollection,
  } from "../../stores/collections";
  import { get } from "svelte/store";
  import CustomDropdown from "../CustomDropdown.svelte";
  import { categoryLabel } from "../../utils/categories";
  import { usesCatalogInstall } from "../../utils/catalogItems";
  import { formatDownloads } from "../../utils/catalogFilters";
  import { sanitizeHtml } from "../../utils/sanitizeHtml";
  import {
    computeInstallationStatus,
    findInstalledRow,
    indexInstalledRows,
  } from "../../utils/installedPlaceholders";
  import {
    dependencySteps,
    installCatalogEntry,
    planCatalogInstall,
    type InstallPlan,
  } from "../../stores/catalogInstall";
  import { thunderstoreEnabled } from "../../stores/catalogFilters";
  import type {
    CatalogSource,
    InstallSourceName,
    ThunderstoreInfo,
  } from "../../stores/modStore";

  // Configure marked options
  marked.use({
    gfm: true,
    breaks: true,
  });

  // Store to track which mods have updates available
  // const updateAvailable = writable<Record<string, boolean>>({});

  const VERSION_CACHE_DURATION = 60 * 60 * 1000;

  interface Props {
    mod: Mod;
    onCheckDependencies?: (
      requirements: { steamodded: boolean; talisman: boolean },
      downloadAction: () => Promise<void>,
    ) => void;
  }

  const { mod, onCheckDependencies }: Props = $props();
  const isDefaultCover = (imageUrl?: string | null) =>
    !!imageUrl && imageUrl.includes("cover.jpg");
  function handleAuxClick(event: MouseEvent) {
    if (event.button === 3) {
      event.preventDefault();
      handleBack();
    }
  }

  const getCategoryName = categoryLabel;

  let installedMods: InstalledMod[] = [];
  let steamoddedVersions = $state<string[]>([]);
  let talismanVersions = $state<string[]>([]);
  let selectedVersion = $state("newest");
  let loadingVersions = $state(false);

  // Computed options for custom dropdowns
  const steamoddedOptions = $derived([
    { value: "newest", label: "latest (could be unstable)" },
    ...steamoddedVersions.map((v) => ({ value: v, label: v })),
  ]);
  const talismanOptions = $derived([
    { value: "newest", label: "latest (could be unstable)" },
    ...talismanVersions.map((v) => ({ value: v, label: v })),
  ]);
  let repoLoading = $state(false);
  let renderedDescription = $state("");
  let descLoading = $state(false);
  const attemptedDescriptions = new Set<string>();
  let isLinux = false;
  let unlistenInstalledMods: (() => void) | null = null;

  function normalizeText(text: string): string {
    return text
      .toLowerCase()
      .replace(/!\[[^\]]*\]\([^)]+\)/g, " ")
      .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
      .replace(/<img[^>]*>/gi, " ")
      .replace(/<[^>]+>/g, " ")
      .replace(/[^a-z0-9]+/g, " ")
      .trim()
      .replace(/\s+/g, " ");
  }

  function hasMeaningfulDescription(
    desc: string | null | void,
    title: string,
  ): boolean {
    if (!desc) return false;
    const trimmed = desc.trim();
    if (!trimmed) return false;
    const normalized = normalizeText(trimmed);
    const normalizedTitle = normalizeText(title);
    if (!normalized || normalized === normalizedTitle) return false;
    if (normalized.startsWith(`what is ${normalizedTitle}`)) return false;
    if (trimmed.length < 60) return false;
    return true;
  }

  function looksLikeFullDescription(desc: string): boolean {
    const trimmed = desc.trim();
    if (!trimmed) return false;
    if (/<[a-z][\s\S]*>/i.test(trimmed)) return true;
    if (/^#{1,6}\s/m.test(trimmed)) return true;
    if (/^\s*(?:[-*+]|\d+\.)\s+/m.test(trimmed)) return true;
    if (/\n\s*\n/.test(trimmed)) return true;
    return trimmed.length >= 200;
  }

  function shouldFetchFullDescription(
    desc: string | null | void,
    title: string,
  ): boolean {
    if (!desc || desc.trim().length === 0) return true;
    if (!hasMeaningfulDescription(desc, title)) return true;
    return !looksLikeFullDescription(desc);
  }

  // Response type for get_mod_details command
  interface ModDetails {
    description: string;
    requires_steamodded: boolean;
    requires_talisman: boolean;
    repo_url: string | null;
    source?: CatalogSource;
    thunderstore?: ThunderstoreInfo | null;
    downloads?: number | null;
  }

  // Track which mods have already been hydrated to avoid duplicate calls
  const hydratedMods = new Set<string>();

  // Unified function to fetch all mod details in a single API call
  async function hydrateModDetails(
    mod: Mod & { _dirName?: string },
  ): Promise<void> {
    if (!mod._dirName) return;
    if (hydratedMods.has(mod.title)) return;

    const modTitle = mod.title;
    const dirName = mod._dirName;
    hydratedMods.add(modTitle);

    try {
      descLoading = true;
      const details = await invoke<ModDetails>("get_mod_details", {
        title: modTitle,
        dirName: dirName,
      });

      // Update description
      if (details.description && details.description.trim().length > 0) {
        setDescription(modTitle, details.description);
        const currentView = $currentModView;
        if (currentView?.title === modTitle) {
          currentModView.set({ ...mod, description: details.description });
        }
      }

      // Thunderstore fields only overwrite when the backend sent them
      const tsFields: Partial<Mod> = {};
      if (details.thunderstore) tsFields.thunderstore = details.thunderstore;
      if (typeof details.downloads === "number") {
        tsFields.downloads = details.downloads;
      }

      // Update requirements and repo URL in store
      modsStore.update((arr) =>
        arr.map((m) =>
          m.title === modTitle
            ? {
                ...m,
                ...tsFields,
                requires_steamodded: details.requires_steamodded,
                requires_talisman: details.requires_talisman,
                repo: details.repo_url ?? m.repo,
              }
            : m,
        ),
      );

      // Update current mod view if still active
      const currentView = $currentModView;
      if (currentView?.title === modTitle) {
        currentModView.set({
          ...mod,
          ...tsFields,
          description: details.description || mod.description,
          requires_steamodded: details.requires_steamodded,
          requires_talisman: details.requires_talisman,
          repo: details.repo_url ?? mod.repo,
        });
      }
    } catch (e) {
      // Fall back to individual calls if unified call fails
      console.warn(
        "get_mod_details failed, falling back to individual calls:",
        e,
      );
      hydratedMods.delete(modTitle);
    } finally {
      descLoading = false;
    }
  }

  async function hydrateRequirements(mod: Mod): Promise<Mod> {
    if (!mod._dirName) return mod;
    if (mod.requires_steamodded || mod.requires_talisman) return mod;
    try {
      const [requiresSteamodded, requiresTalisman] = await invoke<
        [boolean, boolean]
      >("get_mod_requirements", { dirName: mod._dirName });
      if (!requiresSteamodded && !requiresTalisman) return mod;
      modsStore.update((arr) =>
        arr.map((m) =>
          m.title === mod.title
            ? {
                ...m,
                requires_steamodded: requiresSteamodded,
                requires_talisman: requiresTalisman,
              }
            : m,
        ),
      );
      return {
        ...mod,
        requires_steamodded: requiresSteamodded,
        requires_talisman: requiresTalisman,
      };
    } catch (_) {
      return mod;
    }
  }

  async function hydrateRepo(mod: Mod): Promise<void> {
    if (!mod._dirName) return;
    if (mod.repo && mod.repo.trim().length > 0) return;
    const modTitle = mod.title; // Capture title for stale check
    try {
      const repo = await invoke<string | null>("get_mod_repo_url", {
        dirName: mod._dirName,
      });
      if (!repo) return;
      // Only update if this mod is still the active view
      const current = $currentModView;
      if (current?.title !== modTitle) return;
      currentModView.set({ ...mod, repo });
      modsStore.update((arr) =>
        arr.map((m) => (m.title === mod.title ? { ...m, repo } : m)),
      );
    } catch (_) {
      // ignore
    }
  }

  async function handleOpenRepo() {
    if (!mod) return;
    if (mod.repo && mod.repo.trim().length > 0) {
      openExternal(mod.repo).catch(() => {});
      return;
    }
    if (!mod._dirName || repoLoading) return;
    const modTitle = mod.title; // Capture title for stale check
    repoLoading = true;
    try {
      const repo = await invoke<string | null>("get_mod_repo_url", {
        dirName: mod._dirName,
      });
      if (repo && repo.trim().length > 0) {
        // Only update store if this mod is still the active view
        const current = $currentModView;
        if (current?.title === modTitle) {
          currentModView.set({ ...mod, repo });
        }
        modsStore.update((arr) =>
          arr.map((m) => (m.title === mod.title ? { ...m, repo } : m)),
        );
        openExternal(repo).catch(() => {});
      }
    } finally {
      repoLoading = false;
    }
  }

  // Add a local state variable for tracking enabled status
  let isEnabled = $state(true);

  let versionLoadStarted = false;
  let prevModTitle = "";
  let hasCheckedInstallation = false;

  let modsArray: Mod[] = [];
  modsStore.subscribe((m) => (modsArray = m));

  let description: HTMLDivElement;

  const linkCache = new Map<string, internalModLinkData>();

  let modView: HTMLDivElement;
  let descriptionText = $derived(
    mod ? ($descriptionsStore[mod.title] ?? mod.description ?? "") : "",
  );

  interface internalModLinkData {
    isMod: boolean;
    modName: string;
  }

  function isInternalModLink(url: string): internalModLinkData {
    // Quickly check common non-mod paths first
    if (!url || !url.includes("github.com")) {
      return { isMod: false, modName: "" };
    }

    // Exclude specific paths that are not mod repositories
    if (
      url.match(/\.(txt|lua|json|md|png|jpg|jpeg|gif|mp3|ogg|wav)$/) ||
      url.includes("/blob/") ||
      url.includes("/tree/") ||
      url.includes("/wiki") ||
      url.includes("/actions") ||
      url.includes("/issues") ||
      url.includes("/pulls") ||
      url.includes("/commits") ||
      url.includes("/releases") ||
      url.includes("/archive") ||
      url.includes("/compare") ||
      url.includes("/security") ||
      url.includes("/projects")
    ) {
      return { isMod: false, modName: "" };
    }

    // Common patterns for mod links
    const githubModPattern1 = /github\.com\/([^/]+)\/([^/?#]+)(?:$|[?#])/;
    const githubModPattern2 =
      /github\.com\/([^/]+)\/([^/?#]+)(?:\/|\/tree\/|\/blob\/)/;

    // Check if URL matches any pattern
    let match = url.match(githubModPattern1) || url.match(githubModPattern2);

    if (match && match[2]) {
      // Repository name from URL
      const repoName = match[2].toLowerCase();

      // Get mods from the store - avoid subscribers in functions that run during rendering
      let modsArray: Mod[] = [];
      const unsubscribe = modsStore.subscribe((m) => (modsArray = m));
      unsubscribe(); // Important: unsubscribe immediately to prevent memory leaks

      // Find matching mod
      const foundMod = modsArray.find((mod) => {
        // Direct match
        if (mod.title.toLowerCase() === repoName) {
          return true;
        }

        // (removed mistaken await here; Lovely check is performed after installs elsewhere)

        // Match on repo URL
        if (mod.repo && mod.repo.toLowerCase().includes(repoName)) {
          return true;
        }

        // Match with spaces replaced
        const titleDashes = mod.title.toLowerCase().replace(/\s+/g, "-");
        const titleUnderscores = mod.title.toLowerCase().replace(/\s+/g, "_");
        const titleNoSpaces = mod.title.toLowerCase().replace(/\s+/g, "");

        return (
          repoName === titleDashes ||
          repoName === titleUnderscores ||
          repoName === titleNoSpaces
        );
      });

      if (foundMod) {
        return { isMod: true, modName: foundMod.title };
      }
    }

    return { isMod: false, modName: "" };
  }

  async function loadSteamoddedVersions() {
    if (loadingVersions) return;
    try {
      const cached = await invoke<[string[], number]>("load_versions_cache", {
        modType: "steamodded",
      });
      if (cached) {
        const [cachedVers, cachedTs] = cached;
        if (Date.now() - cachedTs * 1000 < VERSION_CACHE_DURATION) {
          steamoddedVersions = cachedVers;
          selectedVersion = "newest";
          if (steamoddedVersions.length > 0) {
            selectedVersion = steamoddedVersions[0];
          }
          cachedVersions.update((c) => ({
            ...c,
            steamodded: cachedVers,
          }));
          return;
        }
      }
    } catch (e) {
      console.error("Version cache check failed:", e);
    }
    loadingVersions = true;
    try {
      const versions: string[] = await invoke("get_steamodded_versions");
      steamoddedVersions = versions;
      selectedVersion = "newest";

      if (versions.length > 0) {
        selectedVersion = versions[0];
      }

      cachedVersions.update((c) => ({ ...c, steamodded: versions }));
      await invoke("save_versions_cache", {
        modType: "steamodded",
        versions,
      });
    } catch (e) {
      console.error("Failed to load Steamodded versions:", e);
      steamoddedVersions = [];
    } finally {
      loadingVersions = false;
    }
  }

  async function loadTalismanVersions() {
    if (loadingVersions) return;
    try {
      const cached = await invoke<[string[], number]>("load_versions_cache", {
        modType: "talisman",
      });
      if (cached) {
        const [cachedVers, cachedTs] = cached;
        if (Date.now() - cachedTs * 1000 < VERSION_CACHE_DURATION) {
          talismanVersions = cachedVers;
          if (cachedVers.length > 0) {
            selectedVersion = cachedVers[0];
          }
          cachedVersions.update((c) => ({
            ...c,
            talisman: cachedVers,
          }));
          return;
        }
      }
    } catch (e) {
      console.error("Version cache check failed:", e);
    }
    loadingVersions = true;
    try {
      const versions: string[] = await invoke("get_talisman_versions");
      talismanVersions = versions;
      if (versions.length > 0) {
        selectedVersion = versions[0];
      }
      cachedVersions.update((c) => ({ ...c, talisman: versions }));
      await invoke("save_versions_cache", {
        modType: "talisman",
        versions,
      });
    } catch (e) {
      console.error("Failed to load Talisman versions:", e);
      talismanVersions = [];
    } finally {
      loadingVersions = false;
    }
  }

  const uninstallMod = async (mod: Mod) => {
    const isCoreMod = ["steamodded", "talisman"].includes(
      mod.title.toLowerCase(),
    );

    try {
      await getAllInstalledMods();
      const installedMod = findInstalledRow(
        mod,
        indexInstalledRows(installedMods),
      );

      if (isCoreMod) {
        // Get dependents
        const dependents = await invoke<string[]>("get_dependents", {
          modName: mod.title,
        });

        // Always show the dialog for core mods
        uninstallDialogStore.set({
          show: true,
          modName: mod.title,
          // Path may be resolved in the dialog if missing
          modPath: installedMod?.path || "",
          dependents,
        });
      } else {
        if (!installedMod) return;
        await invoke("remove_installed_mod", {
          name: installedMod.name,
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

        // Reset update status for this mod
        updateAvailableStore.update((updates) => ({
          ...updates,
          [mod.title]: false,
        }));
      }
    } catch (e) {
      console.error("Failed to uninstall mod:", e);
    }
  };

  const installMod = async (mod: Mod, isUpdate = false) => {
    if (usesCatalogInstall(mod)) {
      await installCatalogEntry(mod, {
        source: pickerSource(mod, isUpdate),
      });
      // Even a failed run may have installed dependencies, so always re-plan
      await getAllInstalledMods();
      refreshTrackedSource();
      planKey = "";
      void loadPlan(mod, selectedSource);
      return;
    }
    // Guard: don't allow re-entrancy while already loading
    if ($loadingStates[mod.title]) return;
    // Set loading immediately to prevent double-clicks
    loadingStates.update((s) => ({ ...s, [mod.title]: true }));

    if (!isLinux) {
      isLinux = await isLinuxPlatform();
    }
    const modToInstall = await hydrateRequirements(mod);

    // Extract the download functionality into a separate async function
    const performDownload = async () => {
      // Set loading state (may be called later from dependency popup)
      loadingStates.update((s) => ({ ...s, [mod.title]: true }));
      try {
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

        // Build dependencies list for the database
        const dependencies = [];
        if (modToInstall.requires_steamodded) dependencies.push("Steamodded");
        if (modToInstall.requires_talisman) dependencies.push("Talisman");

        if (modToInstall.title.toLowerCase() === "steamodded") {
          let installedPath = await invoke<string>(
            "install_steamodded_version",
            { version: selectedVersion },
          );
          const pathExists = await invoke("verify_path_exists", {
            path: installedPath,
          });
          if (!pathExists)
            throw new Error(
              "Installation failed - files not found at destination",
            );
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

          // Reset update status after successful update
          updateAvailableStore.update((updates) => ({
            ...updates,
            [modToInstall.title]: false,
          }));
        } else if (modToInstall.title.toLowerCase() === "talisman") {
          let installedPath = await invoke<string>("install_talisman_version", {
            version: selectedVersion,
          });
          const pathExists = await invoke("verify_path_exists", {
            path: installedPath,
          });
          if (!pathExists)
            throw new Error(
              "Installation failed - files not found at destination",
            );
          await invoke("add_installed_mod", {
            name: modToInstall.title,
            path: installedPath,
            dependencies: [],
            currentVersion: modToInstall.version || "",
          });
          await getAllInstalledMods();
          installationStatus.update((s) => ({
            ...s,
            [modToInstall.title]: true,
          }));

          // Reset update status after successful update
          updateAvailableStore.update((updates) => ({
            ...updates,
            [modToInstall.title]: false,
          }));
        } else {
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

          // Reset update status after successful update
          updateAvailableStore.update((updates) => ({
            ...updates,
            [modToInstall.title]: false,
          }));
        }
      } catch (e) {
        console.error(`Failed to ${isUpdate ? "update" : "install"} mod:`, e);
        const raw = e instanceof Error ? e.message : String(e);
        const onlyUrlMsg = raw.includes("Download URL not reachable")
          ? raw.match(/Download URL not reachable[^"]*/)?.[0] || raw
          : `Failed to ${isUpdate ? "update" : "install"} ${modToInstall.title}: ${raw}`;
        addMessage(onlyUrlMsg, "error");
      } finally {
        loadingStates.update((s) => ({ ...s, [mod.title]: false }));
        void forceRefreshCache();
      }
    };

    // Check dependencies first before doing anything else
    if (modToInstall.requires_steamodded || modToInstall.requires_talisman) {
      // Check Steamodded if required
      const steamoddedInstalled = modToInstall.requires_steamodded
        ? await invoke<boolean>("check_mod_installation", {
            modType: "Steamodded",
          })
        : true;

      // Check Talisman if required
      const talismanInstalled = modToInstall.requires_talisman
        ? await invoke<boolean>("check_mod_installation", {
            modType: "Talisman",
          })
        : true;

      // If any dependency is missing, show the RequiresPopup
      // But skip this check if it's an update, as dependencies should already be installed
      if (
        !isUpdate &&
        ((modToInstall.requires_steamodded && !steamoddedInstalled) ||
          (modToInstall.requires_talisman && !talismanInstalled))
      ) {
        // Clear loading state - performDownload will set it again when/if called
        loadingStates.update((s) => ({ ...s, [mod.title]: false }));
        // Call the handler with the appropriate requirements AND the download action
        onCheckDependencies?.(
          {
            steamodded:
              modToInstall.requires_steamodded && !steamoddedInstalled,
            talisman: modToInstall.requires_talisman && !talismanInstalled,
          },
          performDownload,
        );
        return; // Stop installation
      }
    }

    // If we get here, either no dependencies are required or all are installed
    await performDownload();
  };

  // Function to handle updating the mod
  const updateMod = async (mod: Mod) => {
    await installMod(mod, true);
  };

  function handleMarkdownClick(event: MouseEvent | KeyboardEvent) {
    const anchor = (event.target as HTMLElement).closest("a");
    if (!anchor || !anchor.href) return;

    event.preventDefault();
    event.stopPropagation();

    const internalModName = anchor.getAttribute("data-internal-mod");

    if (internalModName) {
      let modsArray: Mod[] = [];
      modsStore.subscribe((m) => (modsArray = m))();

      const targetMod = modsArray.find((m) => m.title === internalModName);
      if (targetMod) {
        currentModView.set(targetMod);
      }
    } else if (anchor.href.startsWith("http")) {
      openExternal(anchor.href).catch((e) =>
        console.error("Failed to open link:", e),
      );
    }
  }

  // Process links and images in the description
  async function processInternalModLinks() {
    if (!description) return;

    // Auto-link plain URLs in text nodes
    const urlPattern = /https?:\/\/[^\s<>"']+/g;
    const walker = document.createTreeWalker(
      description,
      NodeFilter.SHOW_TEXT,
      null,
    );
    const textNodes: Text[] = [];
    let node: Text | null;
    while ((node = walker.nextNode() as Text | null)) {
      // Skip text nodes inside anchor tags
      if (node.parentElement?.closest("a")) continue;
      if (urlPattern.test(node.textContent || "")) {
        textNodes.push(node);
      }
      // Reset regex lastIndex for next test
      urlPattern.lastIndex = 0;
    }

    for (const textNode of textNodes) {
      const text = textNode.textContent || "";
      const parts: (string | HTMLAnchorElement)[] = [];
      let lastIndex = 0;
      let match: RegExpExecArray | null;

      urlPattern.lastIndex = 0;
      while ((match = urlPattern.exec(text)) !== null) {
        // Add text before the URL
        if (match.index > lastIndex) {
          parts.push(text.slice(lastIndex, match.index));
        }
        // Create anchor element for the URL
        const anchor = document.createElement("a");
        anchor.href = match[0];
        anchor.textContent = match[0];
        parts.push(anchor);
        lastIndex = match.index + match[0].length;
      }

      // Add remaining text after last URL
      if (lastIndex < text.length) {
        parts.push(text.slice(lastIndex));
      }

      // Replace text node with new nodes
      if (
        parts.length > 1 ||
        (parts.length === 1 && typeof parts[0] !== "string")
      ) {
        const fragment = document.createDocumentFragment();
        for (const part of parts) {
          if (typeof part === "string") {
            fragment.appendChild(document.createTextNode(part));
          } else {
            fragment.appendChild(part);
          }
        }
        textNode.parentNode?.replaceChild(fragment, textNode);
      }
    }

    // Process links
    const links = description.querySelectorAll("a");
    for (const link of links) {
      if (link.href.startsWith("http")) {
        let result: internalModLinkData;

        if (linkCache.has(link.href)) {
          result = linkCache.get(link.href)!;
        } else {
          const { isMod, modName } = isInternalModLink(link.href);
          result = { isMod, modName };
          linkCache.set(link.href, result);
        }

        if (result.isMod) {
          link.classList.add("internal-mod-link");
          link.setAttribute("data-internal-mod", result.modName);
        }
      }
    }

    // Process images - add loading states and error handling
    const images = description.querySelectorAll("img");
    for (const img of images) {
      // Skip already processed images
      if (img.dataset.processed) continue;
      img.dataset.processed = "true";

      // Create wrapper for loading state
      const wrapper = document.createElement("div");
      wrapper.className = "desc-img-wrapper";

      // Create loading spinner
      const spinner = document.createElement("div");
      spinner.className = "desc-img-spinner";

      // Insert wrapper before image
      img.parentNode?.insertBefore(wrapper, img);
      wrapper.appendChild(spinner);
      wrapper.appendChild(img);

      // Hide image until loaded
      img.style.opacity = "0";

      // Handle successful load
      img.addEventListener("load", () => {
        spinner.remove();
        img.style.opacity = "1";
      });

      // Handle error - hide the entire wrapper
      img.addEventListener("error", () => {
        wrapper.remove();
      });

      // If image is already loaded (cached), trigger load handler
      if (img.complete && img.naturalWidth > 0) {
        spinner.remove();
        img.style.opacity = "1";
      } else if (img.complete && img.naturalWidth === 0) {
        // Image failed to load
        wrapper.remove();
      }
    }
  }
  const getAllInstalledMods = async () => {
    try {
      installedMods = await fetchCachedMods();
    } catch (error) {
      console.error("Failed to get installed mods:", error);
    }
  };

  const isModInstalled = async (mod: Mod) => {
    if (!mod) return false;

    const rows = await fetchCachedMods();
    const status = Boolean(findInstalledRow(mod, indexInstalledRows(rows)));

    // Update the store outside of the reactive context
    setTimeout(() => {
      installationStatus.update((s) => ({
        ...s,
        [mod.title]: status,
      }));
    }, 0);

    return status;
  };

  // Ensure description is loaded (lazy) for detail view
  async function ensureDescriptionLoaded(m: Mod & { _dirName?: string }) {
    if (!m) return;
    const cached = $descriptionsStore[m.title];
    const current = cached ?? m.description ?? "";
    if (!shouldFetchFullDescription(current, m.title)) return;
    if (attemptedDescriptions.has(m.title)) return;
    const dir = m._dirName as string | void;
    if (!dir) return;
    const modTitle = m.title; // Capture title for stale check
    attemptedDescriptions.add(m.title);
    try {
      descLoading = true;
      const text = await invoke<string>("get_description_cached_or_remote", {
        title: m.title,
        dirName: dir,
      });
      if (text && text.trim().length > 0) {
        setDescription(m.title, text);
        // Only update currentModView if this mod is still the active view
        const currentView = $currentModView;
        if (currentView?.title === modTitle) {
          currentModView.set({ ...m, description: text });
        }
      }
    } catch (_) {
      // ignore
    } finally {
      descLoading = false;
    }
  }

  // This effect handles the description rendering
  $effect(() => {
    const m = mod as Mod & { _dirName?: string };
    const desc = descriptionText;

    // Clear rendered description immediately when mod changes to show loading state
    if (!desc) {
      renderedDescription = "";
    }

    if (m) {
      // Use unified API call to fetch all mod details at once
      hydrateModDetails(m);
    }
    if (desc) {
      Promise.resolve(marked(desc)).then((result) => {
        renderedDescription = sanitizeHtml(result);
      });
    }
  });

  // Watch for changes to renderedDescription separately
  $effect(() => {
    if (renderedDescription) {
      // Use setTimeout to move to next microtask
      setTimeout(() => {
        processInternalModLinks();
      }, 0);
    }
  });

  function handleBack() {
    currentModView.set(null);
  }

  function isEditableTarget(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLElement &&
      (target.isContentEditable ||
        target.closest("input, textarea, select") !== null)
    );
  }

  function handleWindowKeydown(e: KeyboardEvent) {
    // Escape belongs to the topmost popup while one is open
    if (e.defaultPrevented || isAnyPopupOpen()) return;
    if (
      e.key === "Escape" ||
      (e.key === "Backspace" && !isEditableTarget(e.target))
    ) {
      handleBack();
    }
  }

  onMount(async () => {
    isLinux = await isLinuxPlatform();
    window.addEventListener("auxclick", handleAuxClick);

    // Initial load of installed mods
    await getAllInstalledMods();

    // Pre-fetch mod requirements in background so download doesn't block on API
    if (mod && !mod.requires_steamodded && !mod.requires_talisman) {
      hydrateRequirements(mod).catch(() => {});
    }

    // Check if the current mod is installed
    if (mod && !hasCheckedInstallation) {
      hasCheckedInstallation = true;
      setTimeout(() => {
        isModInstalled(mod);
      }, 0);
    }

    // Keep Mod View install state in sync with backend changes.
    try {
      unlistenInstalledMods = await listen(
        "installed-mods-changed",
        async () => {
          try {
            await forceRefreshCache();
            if (mod) {
              await isModInstalled(mod);
            }
          } catch (_) {
            // ignore refresh errors
          }
        },
      );
    } catch (_) {
      // ignore listen errors
    }
  });

  // Handle mod changes from currentModView
  $effect(() => {
    const currentMod = untrack(() => $currentModView);

    if (currentMod) {
      // Check if this is a new mod
      if (!hasCheckedInstallation || (mod && mod.title !== currentMod.title)) {
        hasCheckedInstallation = true;

        // Move installation check outside reactive context
        setTimeout(() => {
          isModInstalled(currentMod);
        }, 0);
      }
    }
  });

  // Handle loading of version data for special mods
  $effect(() => {
    const currentModTitle = mod?.title?.toLowerCase();
    if (
      currentModTitle === "steamodded" &&
      currentModTitle !== prevModTitle &&
      !versionLoadStarted
    ) {
      prevModTitle = currentModTitle;
      versionLoadStarted = true;

      // Move version loading outside reactive context
      setTimeout(() => {
        loadSteamoddedVersions().then(() => {
          versionLoadStarted = false;
        });
      }, 0);
    } else if (
      currentModTitle === "talisman" &&
      currentModTitle !== prevModTitle &&
      !versionLoadStarted
    ) {
      prevModTitle = currentModTitle;
      versionLoadStarted = true;

      // Move version loading outside reactive context
      setTimeout(() => {
        loadTalismanVersions().then(() => {
          versionLoadStarted = false;
        });
      }, 0);
    }
  });

  onDestroy(async () => {
    window.removeEventListener("auxclick", handleAuxClick);
    try {
      if (typeof unlistenInstalledMods === "function") {
        unlistenInstalledMods();
      }
    } catch (_) {
      // ignore
    }
    cachedVersions.set({ steamodded: [], talisman: [] });

    // Ensure installation status is updated before component unmounts
    if ($currentCategory === "Installed Mods") {
      await getAllInstalledMods();
      const status = computeInstallationStatus(modsArray, installedMods);
      installationStatus.update((s) => ({ ...s, ...status }));
    }
  });

  async function checkModEnabled(modName: string) {
    try {
      const enabled = await invoke<boolean>("is_mod_enabled", {
        modName,
      });

      modEnabledStore.update((enabledMods: Record<string, boolean>) => ({
        ...enabledMods,
        [modName]: enabled,
      }));

      // Update local variable for reactive binding
      isEnabled = enabled;
    } catch (error) {
      console.error(`Failed to check if mod ${modName} is enabled:`, error);
      // Default to enabled on error
      modEnabledStore.update((enabledMods: Record<string, boolean>) => ({
        ...enabledMods,
        [modName]: true,
      }));
      isEnabled = true;
    }
  }

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
    } catch (error) {
      console.error(`Failed to toggle mod ${mod.title} enabled state:`, error);
    }
  }

  $effect(() => {
    if ($installationStatus[mod.title]) {
      checkModEnabled(mod.title);
    }
  });

  // Thunderstore side: source picker for merged mods and the dependency plan
  let isMerged = $derived(mod.source === "merged" && Boolean(mod.thunderstore));
  let trackedSource = $state<InstallSourceName | null>(null);
  let chosenSource = $state<InstallSourceName | null>(null);
  let selectedSource = $derived<InstallSourceName>(
    chosenSource ??
      trackedSource ??
      (mod.source === "thunderstore" ||
      (mod.source === "merged" && $thunderstoreEnabled && !mod.deprecated)
        ? "thunderstore"
        : "index"),
  );
  let plan = $state<InstallPlan | null>(null);
  let planError = $state<string | null>(null);
  let planLoading = $state(false);
  let planKey = "";
  let downloadsLabel = $derived(formatDownloads(mod.downloads));
  let packageUrl = $derived(mod.thunderstore?.package_url ?? "");
  let planDependencies = $derived(plan ? dependencySteps(plan) : []);
  let pickerModKey = "";

  // The view is reused when another mod opens, start each one on its default source
  $effect(() => {
    const key = mod._dirName ?? mod.title;
    if (key === pickerModKey) return;
    pickerModKey = key;
    chosenSource = null;
    plan = null;
    planError = null;
  });

  /** Explicit source for the backend, updates keep the tracked source unless the user switched */
  function pickerSource(m: Mod, isUpdate: boolean): InstallSourceName | null {
    if (m.source !== "merged") return null;
    if (chosenSource) return chosenSource;
    if (isUpdate || trackedSource) return null;
    return $thunderstoreEnabled ? null : "index";
  }

  function refreshTrackedSource() {
    const row = findInstalledRow(mod, indexInstalledRows(installedMods));
    trackedSource = row
      ? row.source === "thunderstore"
        ? "thunderstore"
        : "index"
      : null;
  }

  async function loadPlan(m: Mod, source: InstallSourceName) {
    const key = `${m._dirName}|${source}`;
    if (key === planKey) return;
    planKey = key;
    planLoading = true;
    planError = null;
    try {
      const next = await planCatalogInstall(m, {
        source: m.source === "merged" ? source : null,
      });
      if (planKey === key) plan = next;
    } catch (error) {
      if (planKey === key) {
        plan = null;
        planError = error instanceof Error ? error.message : String(error);
      }
    } finally {
      if (planKey === key) planLoading = false;
    }
  }

  async function switchSource() {
    if (!chosenSource || chosenSource === trackedSource) return;
    await installMod(mod, true);
  }

  function openThunderstore() {
    if (packageUrl) openExternal(packageUrl).catch(() => {});
  }

  $effect(() => {
    const m = mod;
    const source = selectedSource;
    if (usesCatalogInstall(m)) void loadPlan(m, source);
  });

  $effect(() => {
    void $installationStatus[mod.title];
    getAllInstalledMods().then(refreshTrackedSource);
  });
</script>

<svelte:window onkeydown={handleWindowKeydown} />

<div
  class="mod-view default-scrollbar"
  transition:fade={{ duration: 300, easing: cubicOut }}
  bind:this={modView}
>
  <div class="mod-content">
    <div class="header-container">
      <div class="header">
        <button class="back-button" onclick={handleBack}>
          <ArrowLeft size={20} /> <span>Back</span>
        </button>
      </div>
    </div>

    <h2>{mod.title}</h2>
    <div class="content-grid">
      <div class="left-column">
        <div class="image-container">
          {#if !isDefaultCover(mod.image)}
            <button
              class="image-button"
              aria-label={`View full size image of ${mod.title}`}
            >
              <LazyImage
                src={mod.image}
                fallbackSrc={mod.imageFallback}
                alt={mod.title}
                cacheTitle={mod.title}
                cacheId={mod._dirName}
                remoteSrc={mod.thumbnailUrl}
                hasThumbnail={mod._hasThumbnail !== false}
              />
            </button>
          {:else}
            <LazyImage
              src={mod.image}
              fallbackSrc={mod.imageFallback}
              alt={mod.title}
              cacheTitle={mod.title}
              cacheId={mod._dirName}
              remoteSrc={mod.thumbnailUrl}
              hasThumbnail={mod._hasThumbnail !== false}
            />
          {/if}
        </div>

        <div class="button-container">
          <!-- Enable/Disable toggle button - MOVED TO FIRST POSITION -->
          {#if $installationStatus[mod.title]}
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

          <button
            class="collection-button"
            title="Collections"
            onclick={() => openCollectionPicker(mod.title, mod.id)}
          >
            <Layers size={18} />
          </button>

          {#if $installationStatus[mod.title] && $updateAvailableStore[mod.title] && !mod.orphaned}
            <!-- Update button (when installed and update available) -->
            <button
              class="update-button"
              onclick={() => updateMod(mod)}
              disabled={$loadingStates[mod.title]}
            >
              {#if $loadingStates[mod.title]}
                <div class="spinner"></div>
              {:else}
                <RefreshCw size={18} />
                Update Mod
              {/if}
            </button>
          {:else}
            <!-- Regular download/installed button -->
            <button
              class="download-button"
              class:installed={$installationStatus[mod.title]}
              disabled={$installationStatus[mod.title] ||
                $loadingStates[mod.title]}
              onclick={() => installMod(mod)}
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
            <button
              class="delete-button"
              title="Remove Mod"
              onclick={() => uninstallMod(mod)}
            >
              <Trash2 size={18} />
            </button>
          {/if}
        </div>

        {#if isMerged}
          <div class="source-picker" role="group" aria-label="Install source">
            <span class="source-label">Source</span>
            {#each ["thunderstore", "index"] as const as option (option)}
              <button
                type="button"
                class="source-option"
                class:active={selectedSource === option}
                aria-pressed={selectedSource === option}
                disabled={$loadingStates[mod.title]}
                onclick={() => (chosenSource = option)}
              >
                {option === "thunderstore" ? "Thunderstore" : "Index"}
                {#if trackedSource === option}
                  <span class="source-current">installed</span>
                {/if}
              </button>
            {/each}
          </div>
          {#if $installationStatus[mod.title] && trackedSource && chosenSource && chosenSource !== trackedSource}
            <button
              class="switch-source-button"
              onclick={switchSource}
              disabled={$loadingStates[mod.title]}
            >
              <RefreshCw size={16} />
              Switch to {chosenSource === "thunderstore"
                ? "Thunderstore"
                : "the index"}
            </button>
          {/if}
        {/if}

        {#if mod.title.toLowerCase() === "talisman" && !$installationStatus[mod.title]}
          <div class="version-selector">
            {#if loadingVersions}
              <div class="loading-text">Loading versions...</div>
            {:else if talismanVersions.length === 0}
              <div class="loading-text">No versions available</div>
            {:else}
              <CustomDropdown
                options={talismanOptions}
                bind:value={selectedVersion}
                disabled={$loadingStates[mod.title]}
                placeholder="Select version"
              />
            {/if}
          </div>
        {/if}
        {#if mod.title.toLowerCase() === "steamodded" && !$installationStatus[mod.title]}
          <div class="version-selector">
            {#if loadingVersions}
              <div class="loading-text">Loading versions...</div>
            {:else if steamoddedVersions.length === 0}
              <div class="loading-text">No versions available</div>
            {:else}
              <CustomDropdown
                options={steamoddedOptions}
                bind:value={selectedVersion}
                disabled={$loadingStates[mod.title]}
                placeholder="Select version"
              />
            {/if}
          </div>
        {/if}
        <div class="mod-stats">
          <span><User size={16} /> {mod.publisher}</span>
          {#if mod.thunderstore?.owner && mod.thunderstore.owner !== mod.publisher}
            <span title="Thunderstore owner">
              <User size={16} />
              {mod.thunderstore.owner}
            </span>
          {/if}
          {#if downloadsLabel}
            <span title="Downloads on Thunderstore">
              <ArrowDownToLine size={16} />
              {downloadsLabel}
            </span>
          {/if}
          <LastUpdated value={mod.last_updated} />
        </div>
        {#if mod.deprecated || mod.aiGenerated || mod.nsfw}
          <div class="flag-badges">
            {#if mod.deprecated}
              <span class="flag-badge deprecated">Deprecated</span>
            {/if}
            {#if mod.aiGenerated}
              <span class="flag-badge ai">AI Generated</span>
            {/if}
            {#if mod.nsfw}
              <span class="flag-badge nsfw">NSFW</span>
            {/if}
          </div>
        {/if}
        {#if packageUrl}
          <button onclick={openThunderstore} class="repo-button ts-button">
            <ExternalLink size={16} /> View on Thunderstore
          </button>
        {/if}
        {#if mod.repo || (mod._dirName && mod.source !== "thunderstore")}
          <button
            onclick={handleOpenRepo}
            class="repo-button"
            disabled={repoLoading}
          >
            <Github size={16} /> Repository
          </button>
        {/if}

        {#if usesCatalogInstall(mod)}
          <div class="dependencies-section" data-testid="plan-dependencies">
            <h3>Dependencies</h3>
            {#if planLoading && !plan}
              <p class="dep-empty">Checking dependencies...</p>
            {:else if planError}
              <p class="dep-empty">{planError}</p>
            {:else if plan}
              <ul class="dep-list">
                {#each planDependencies as step (step.catalog_id)}
                  <li>
                    <span class="dep-name">{step.title}</span>
                    <span class="dep-state missing">
                      {step.installed_version
                        ? `update to ${step.version}`
                        : `installs ${step.version || "latest"}`}
                    </span>
                  </li>
                {/each}
                {#each plan.satisfied as dep (dep.full_name)}
                  <li>
                    <span class="dep-name">{dep.title}</span>
                    <span class="dep-state ok">installed</span>
                  </li>
                {/each}
                {#if plan.steamodded}
                  <li>
                    <span class="dep-name">
                      Steamodded{plan.steamodded.min_tag
                        ? ` ${plan.steamodded.min_tag}+`
                        : ""}
                    </span>
                    <span
                      class="dep-state"
                      class:ok={plan.steamodded.satisfied}
                      class:missing={!plan.steamodded.satisfied}
                    >
                      {plan.steamodded.satisfied
                        ? "installed"
                        : plan.steamodded.installed
                          ? "too old"
                          : "missing"}
                    </span>
                  </li>
                {/if}
                {#if plan.talisman}
                  <li>
                    <span class="dep-name">Talisman</span>
                    <span
                      class="dep-state"
                      class:ok={plan.talisman.installed}
                      class:missing={!plan.talisman.installed}
                    >
                      {plan.talisman.installed ? "installed" : "missing"}
                    </span>
                  </li>
                {/if}
                {#if plan.lovely}
                  <li>
                    <span class="dep-name">
                      Lovely {plan.lovely.min_version}+
                    </span>
                    <span
                      class="dep-state"
                      class:ok={plan.lovely.satisfied}
                      class:missing={!plan.lovely.satisfied}
                    >
                      {plan.lovely.satisfied ? "installed" : "update needed"}
                    </span>
                  </li>
                {/if}
              </ul>
              {#if planDependencies.length === 0 && plan.satisfied.length === 0 && !plan.steamodded && !plan.talisman && !plan.lovely}
                <p class="dep-empty">No dependencies</p>
              {/if}
              {#each plan.warnings as warning, i (i)}
                <p class="dep-warning">{warning}</p>
              {/each}
            {/if}
          </div>
        {/if}

        {#if mod.categories && mod.categories.length > 0}
          <div class="categories-section">
            <h3>Categories</h3>
            <div class="category-tags">
              {#each mod.categories as category}
                <button
                  class="category-tag"
                  onclick={() => {
                    currentPage.set(1);
                    currentModView.set(null);
                    currentCategory.set(getCategoryName(category));
                    setTimeout(() => {
                      const modsContainer = document.querySelector(
                        ".mods-scroll-container",
                      );
                      if (modsContainer) {
                        modsContainer.scrollTo({
                          top: 0,
                          behavior: "smooth",
                        });
                      } else {
                        // Fallback to window scroll
                        window.scrollTo({
                          top: 0,
                          behavior: "smooth",
                        });
                      }
                    }, 50);
                  }}
                >
                  {getCategoryName(category)}
                </button>
              {/each}
            </div>
          </div>
        {/if}
      </div>
      <div class="right-column">
        <div
          class="description"
          role="button"
          bind:this={description}
          tabindex="0"
          onclick={handleMarkdownClick}
          onkeydown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              handleMarkdownClick(e);
            }
          }}
        >
          {#if descLoading}
            <div class="desc-loading-full">
              <div class="loading-spinner"></div>
              <span>Loading description...</span>
            </div>
          {:else}
            <div
              class="desc-content"
              transition:fade={{ duration: 300, easing: cubicOut }}
            >
              <!-- eslint-disable-next-line svelte/no-at-html-tags -->
              {@html renderedDescription}
            </div>
          {/if}
        </div>
      </div>
    </div>
  </div>
</div>

<style>
  .source-picker {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin-top: 0.75rem;
    flex-wrap: wrap;
  }

  .source-label {
    font-family: "M6X11", sans-serif;
    color: var(--ui-text);
    opacity: 0.8;
    margin-right: 0.2rem;
  }

  .source-option {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    background: var(--ui-mod-chip-bg);
    border: 2px solid var(--ui-mod-chip-border);
    color: var(--ui-mod-chip-text);
    font-family: "M6X11", sans-serif;
    font-size: 0.95rem;
    padding: 0.35rem 0.7rem;
    border-radius: 6px;
    cursor: pointer;
  }

  .source-option.active,
  .source-option:hover:not(:disabled) {
    background: var(--ui-mod-chip-active-bg);
    color: var(--ui-mod-chip-active-text);
  }

  .source-current {
    font-size: 0.7rem;
    text-transform: uppercase;
    opacity: 0.8;
  }

  .switch-source-button {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    margin-top: 0.6rem;
    padding: 0.5rem 0.9rem;
    border: none;
    border-radius: 6px;
    background: var(--ui-info);
    color: var(--ui-text);
    outline: var(--ui-info-strong) solid 2px;
    font-family: "M6X11", sans-serif;
    cursor: pointer;
  }

  .switch-source-button:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .flag-badges {
    display: flex;
    gap: 0.4rem;
    margin-top: 0.6rem;
    flex-wrap: wrap;
  }

  .flag-badge {
    font-family: "M6X11", sans-serif;
    font-size: 0.8rem;
    padding: 0.25rem 0.5rem;
    border-radius: 4px;
    text-transform: uppercase;
    color: #fff;
    border: 1px solid rgba(255, 255, 255, 0.25);
  }

  .flag-badge.deprecated {
    background: rgba(192, 64, 56, 0.92);
  }

  .flag-badge.ai {
    background: rgba(121, 110, 158, 0.95);
  }

  .flag-badge.nsfw {
    background: rgba(160, 40, 90, 0.95);
  }

  .ts-button {
    margin-bottom: 0.5rem;
  }

  .dependencies-section {
    margin-top: 1.5rem;
  }

  .dependencies-section h3 {
    color: var(--ui-heading);
    font-size: 1.5rem;
    margin-bottom: 0.6rem;
  }

  .dep-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .dep-list li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.4rem 0.6rem;
    border-radius: 6px;
    background: var(--ui-mod-chip-bg);
    border: 1px solid var(--ui-mod-chip-border);
    color: var(--ui-mod-chip-text);
  }

  .dep-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dep-state {
    font-family: "M6X11", sans-serif;
    font-size: 0.85rem;
    flex-shrink: 0;
  }

  .dep-state.ok {
    color: var(--ui-success-strong);
  }

  .dep-state.missing {
    color: var(--ui-warning);
  }

  .dep-empty,
  .dep-warning {
    color: var(--ui-text);
    opacity: 0.8;
    margin: 0.4rem 0 0;
  }

  .dep-warning {
    color: var(--ui-warning);
    opacity: 1;
  }

  :global(.description img) {
    max-width: 100%;
    max-height: 300px;
    width: auto;
    height: auto;
    object-fit: contain;
    display: block;
    -webkit-user-drag: none;
    user-select: none;
    -webkit-user-select: none;
    pointer-events: none;
  }

  .toggle-button {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 1rem;
    border: none;
    border-radius: 6px;
    font-size: 1rem;
    cursor: pointer;
    transition: all 0.2s ease;
    font-family: "M6X11", sans-serif;
    min-height: 48px;
  }

  .toggle-button.enabled {
    background: var(--ui-success-strong); /* Bright green when enabled */
    color: var(--ui-text);
  }

  .toggle-button.disabled {
    background: var(--ui-neutral); /* Gray when disabled */
    color: var(--ui-text);
  }

  .toggle-button:hover.enabled {
    background: var(--ui-success-strong-hover); /* Lighter green on hover */
    transform: translateY(-2px);
  }

  .toggle-button:hover.disabled {
    background: var(--ui-neutral-hover); /* Lighter gray on hover */
    transform: translateY(-2px);
  }

  .toggle-button:active {
    transform: translateY(1px);
  }

  .collection-button {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 1rem;
    border: none;
    border-radius: 6px;
    font-size: 1rem;
    cursor: pointer;
    transition: all 0.2s ease;
    font-family: "M6X11", sans-serif;
    min-height: 48px;
    background: var(--ui-info-strong);
    color: var(--ui-text);
  }

  .collection-button:hover {
    background: var(--ui-info-strong-hover);
    transform: translateY(-2px);
  }

  .collection-button:active {
    transform: translateY(1px);
  }

  .categories-section {
    margin-top: 1.5rem;
    padding: 0.75rem;
    background: var(--ui-glass-weak);
    border-radius: 6px;
  }

  .categories-section h3 {
    margin: 0 0 0.7rem 0;
    font-size: 1.2rem;
    color: var(--ui-text);
    text-align: center;
  }

  .category-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem; /* Increased gap between tags */
    width: 100%;
    justify-content: center;
  }

  .category-tag {
    background: var(--ui-glass); /* Transparent background */
    color: var(--ui-text);
    border: 1px solid var(--ui-glass-border); /* Subtle border */
    border-radius: 6px;
    padding: 0.5rem 1rem; /* Larger padding for bigger tags */
    font-size: 1.1rem; /* Larger font size */
    cursor: pointer;
    transition: all 0.2s ease;
    font-family: "M6X11", sans-serif;
    backdrop-filter: blur(8px); /* Add blur effect */
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
  }

  .category-tag:hover {
    background: var(--ui-glass-strong); /* Slightly more visible on hover */
    transform: translateY(-2px);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
  }

  .category-tag:active {
    transform: translateY(1px);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.1);
  }

  .mod-view {
    position: fixed;
    top: 0;
    right: 0;
    width: 100%;
    height: 100%;
    /* background: linear-gradient(to bottom, #393646, #4a4458); */
    background: var(--ui-danger-overlay);
    backdrop-filter: blur(20px);
    z-index: 1000;
    overflow-y: auto;
    font-family: "M6X11", sans-serif;
  }

  .mod-content {
    position: relative;

    /* max-width: 1000px; */
    padding: 3rem;
    color: var(--ui-text);
    font-family: "M6X11", sans-serif;
  }

  .image-button {
    padding: 0;
    margin: 0;
    border: none;
    background: none;
    cursor: default; /* do not show pointer over thumbnail */
    width: 100%;
    height: 100%;
    display: block;
    line-height: 0; /* remove any spacing */
    font-size: 0; /* remove any spacing */
  }

  h2 {
    margin-bottom: 2rem;
    font-size: 1.8rem;
  }

  .content-grid {
    display: grid;
    grid-template-columns: 350px 1fr;
    gap: 3rem;
  }

  .image-container {
    border-radius: 8px;
    height: 250px;
    overflow: hidden;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
  }

  /* Image display is managed by LazyImage; keep container-only styles */
  .image-container {
    cursor: default;
  }

  /* Subtle zoom-in on hover for thumbnail (style child component via :global) */
  .image-container :global(.lazy-image img) {
    transition: transform 180ms ease-out;
    will-change: transform;
  }
  .image-container:hover :global(.lazy-image img) {
    transform: scale(1.04);
  }

  .button-container {
    display: flex;
    gap: 0.5rem;
    margin: 1rem 0;
  }

  .download-button,
  .update-button {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.5rem;
    padding: 1rem;
    border: none;
    border-radius: 6px;
    font-size: 1rem;
    cursor: pointer;
    transition: all 0.2s ease;
    font-family: "M6X11", sans-serif;
    /* Fixed height to prevent resizing */
    min-height: 48px;
  }

  .download-button {
    background: var(--ui-success);
    color: var(--ui-text);
  }

  .update-button {
    background: var(--ui-info); /* Bright blue color */
    color: var(--ui-text);
  }

  .update-button:hover:not(:disabled) {
    background: var(--ui-info-hover); /* Lighter blue on hover */
    transform: translateY(-2px);
  }

  .update-button:active:not(:disabled) {
    transform: translateY(1px);
  }

  .spinner {
    border: 2px solid var(--ui-glass-border);
    border-top: 2px solid var(--ui-text);
    border-radius: 50%;
    width: 16px;
    height: 16px;
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    0% {
      transform: rotate(0deg);
    }
    100% {
      transform: rotate(360deg);
    }
  }

  .download-button:hover:not(.installed) {
    background: var(--ui-success-hover);
    transform: translateY(-2px);
  }

  .download-button.installed {
    background: var(--ui-neutral);
    cursor: not-allowed;
  }

  .download-button:active:not(.installed) {
    transform: translateY(1px);
  }

  .delete-button {
    padding: 0.75rem;
    background: var(--ui-danger);
    color: var(--ui-text);
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .delete-button:hover {
    background: var(--ui-danger-hover);
    transform: translateY(-2px);
  }

  .mod-stats {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    font-size: 1.1rem;
    padding: 1rem;
    background: var(--ui-glass);
    border-radius: 6px;
    justify-content: center;
    align-items: center;
  }

  .mod-stats :global(span) {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--ui-text);
  }

  :global(.description img) {
    max-width: 100%;
    max-height: 300px;
    width: auto;
    height: auto;
    object-fit: contain;
    display: block;
    -webkit-user-drag: none;
    user-select: none;
    -webkit-user-select: none;
    pointer-events: none;
  }

  .description {
    font-size: 1.2rem;
    line-height: 1;
    color: var(--ui-text);
    background: var(--ui-glass-weak);
    padding: 1.25rem;
    border-radius: 6px;
    width: 50rem;
    line-height: 1.5;
  }

  /* Improved inline code styling */
  .description :global(code) {
    background: var(--ui-panel-muted);
    color: var(--ui-text);
    padding: 0.2em 0.4em;
    border-radius: 3px;
    font-family: "Consolas", "Monaco", "Menlo", monospace;
    font-size: 0.75em;
  }

  /* Improved code block styling */
  .description :global(pre) {
    background: var(--ui-panel-muted-strong);
    padding: 1em;
    border-radius: 6px;
    overflow-x: auto;
    margin: 1em 0;
    border: 1px solid var(--ui-panel-muted-border);
  }

  /* Style code within pre blocks differently than inline code */
  .description :global(pre code) {
    background: transparent;
    padding: 0;
    color: var(--ui-text);
    display: block;
    line-height: 1.5;
    white-space: pre;
  }

  /* Add syntax highlighting colors */
  .description :global(.token.keyword),
  .description :global(.token.operator) {
    color: #ff7b72;
  }

  .description :global(.token.string),
  .description :global(.token.char) {
    color: #a5d6ff;
  }

  .description :global(.token.function),
  .description :global(.token.method) {
    color: #d2a8ff;
  }

  .description :global(.token.number) {
    color: #f8c555;
  }

  .description :global(.token.comment) {
    color: #8b949e;
    font-style: italic;
  }

  .description :global(.token.boolean),
  .description :global(.token.constant) {
    color: #79c0ff;
  }

  .header-container {
    position: absolute;
    top: 0;
    left: 0;
    height: 100%;
    width: 100%;

    z-index: 999;

    pointer-events: none;
  }

  .back-button {
    position: relative;
    /* top: 1rem;
		left: 1rem; */
    display: flex;
    align-items: center;
    gap: 0.5rem;
    background: var(--ui-glass);
    border: none;
    color: var(--ui-text);
    padding: 0.5rem 1rem;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.2s ease;
    font-family: "M6X11", sans-serif;
    font-size: 1rem;
    z-index: 100;

    pointer-events: auto;

    backdrop-filter: blur(20px) brightness(0.7);
  }

  .back-button:hover {
    background: var(--ui-glass-strong);
    transform: translateX(-5px);
  }

  .header {
    display: flex;
    justify-content: space-between;
    align-items: center;

    box-sizing: border-box;
    position: sticky;

    top: 1rem;
    width: 100%;
    height: 2.5rem;

    padding: 0 1rem;
  }

  .description :global(h1),
  .description :global(h2),
  .description :global(h3),
  .description :global(h4) {
    margin-bottom: 0.5em;
    color: var(--ui-text);
  }

  .description :global(p) {
    margin-bottom: 1em;
  }

  .description :global(ul),
  .description :global(ol) {
    margin-left: 1.5em;
    margin-bottom: 1em;
  }

  .description :global(li) {
    margin-bottom: 0.5em;
  }

  .description :global(a) {
    color: var(--ui-success);
    text-decoration: none;
    cursor: pointer;
    pointer-events: auto;
  }

  .description :global(a.internal-mod-link) {
    /* Use Balatro's gold color for internal mod links */
    color: var(--ui-accent) !important;
    position: relative;
  }

  .description :global(a.internal-mod-link::after) {
    display: inline-block;
    margin-left: 3px;
    transform: rotate(-45deg);
    font-weight: bold;
  }

  .description :global(a.internal-mod-link:hover) {
    text-decoration: underline;
    filter: brightness(1.2);
  }

  .description :global(a.internal-mod-link:hover::before) {
    content: "Open in Mod Manager";
    position: absolute;
    bottom: -35px;
    left: 0;
    background: var(--ui-backdrop-strong);
    color: white;
    padding: 4px 8px;
    border-radius: 4px;
    font-size: 0.8em;
    white-space: nowrap;
    z-index: 10;
  }

  .description :global(a:hover) {
    text-decoration: underline;
    z-index: 10;
  }

  .description :global(blockquote) {
    border-left: 3px solid var(--ui-success);
    margin: 1em 0;
    padding-left: 1em;
    color: var(--ui-text-soft);
  }

  .description :global(a) {
    -webkit-user-drag: none;
    user-select: none;
    -moz-user-select: none;
    -webkit-user-select: none;
    -ms-user-select: none;
  }

  .delete-button:active {
    transform: translateY(1px);
  }
  /* Image elements live inside LazyImage, so no direct img rules here */

  /* .image-container .clickable { */
  /* 	cursor: pointer; */
  /* } */

  @media (max-width: 1360px) {
    .content-grid {
      grid-template-columns: 1fr;
    }
    .image-container {
      width: 100%;
      height: 350px;
    }

    .image-button {
      height: 100%;
    }

    .right-column {
      bottom: 2rem;
      position: relative;
    }

    .mod-content {
      width: 100%;
      max-width: 100%;
      box-sizing: border-box;
    }

    .right-column {
      display: flex;
      flex-direction: column;
      align-items: center;
    }
  }

  .download-button:disabled,
  .update-button:disabled {
    opacity: 0.8;
    cursor: not-allowed;
  }

  .version-selector {
    margin-bottom: 1rem;
    width: 100%;
  }

  .loading-text {
    /* width: 100%; */
    padding: 0.75rem;
    background: var(--ui-danger-overlay);
    color: var(--ui-text);
    border: 1px solid var(--ui-danger-overlay-border);
    border-radius: 6px;
    font-family: "M6X11", sans-serif;
    font-size: 1rem;
    text-align: center;
  }

  .repo-button {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    height: 3rem;
    padding: 0.75rem 1.5rem;
    background: var(--ui-code-bg);
    color: var(--ui-text);
    border: none;
    outline: var(--ui-code-outline) solid 2px;
    border-radius: 4px;
    font-family: "M6X11", sans-serif;
    font-size: 1rem;
    cursor: pointer;
    transition: all 0.2s ease;
    text-decoration: none;
    margin-top: 1rem;
    justify-content: center;
  }

  .repo-button:hover {
    background: var(--ui-code-bg-hover);
    transform: translateY(-2px);
  }

  .description :global(a) {
    color: var(--ui-success);
    text-decoration: none;
    cursor: pointer;
    transition: all 0.2s ease;
  }

  .description :global(a:hover) {
    text-decoration: underline;
    filter: brightness(1.2);
  }

  :global([data-platform="linux"]) .category-tag {
    backdrop-filter: none;
    background: var(--ui-glass);
  }

  :global([data-platform="linux"]) .mod-view {
    backdrop-filter: none;
    background: var(--ui-danger-overlay-strong);
    position: absolute;
    inset: 0;
    border-radius: 6px;
    max-height: 100%;
    overflow-y: auto;
  }

  :global([data-platform="linux"]) .back-button {
    backdrop-filter: none;
    background: var(--ui-glass);
  }

  .desc-loading-full {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1.5rem;
    padding: 4rem 1rem;
    color: var(--ui-text-soft);
    font-size: 1.5rem;
  }

  .desc-loading-full .loading-spinner {
    width: 48px;
    height: 48px;
    border: 6px solid var(--ui-text-soft);
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  :global(.description .desc-img-wrapper) {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 60px;
    margin: 0.5rem 0;
  }

  :global(.description .desc-img-spinner) {
    position: absolute;
    width: 24px;
    height: 24px;
    border: 3px solid var(--ui-text-soft);
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  :global(.description .desc-img-wrapper img) {
    transition: opacity 0.2s ease;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
