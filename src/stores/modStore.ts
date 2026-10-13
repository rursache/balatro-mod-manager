import { writable, get, type Writable } from "svelte/store";
import {
  safeLocalStorageSet,
  monitorStorageQuota,
} from "../utils/storage-quota";
import { ensureCatalogSchema } from "../utils/catalogSchema";

export interface Mod {
  id?: string;
  title: string;
  description: string;
  image: string;
  imageFallback?: string;
  /** Index thumbnail URL, downloaded into the disk cache once the card is on screen */
  thumbnailUrl?: string;
  // Internal optional fields used by views/cache
  _dirName?: string;
  _installedPath?: string;
  _hasThumbnail?: boolean;
  /** True when this mod is tracked locally but no longer present in the remote index. */
  orphaned?: boolean;
  /** Version recorded at install time, shown for mods the index no longer lists */
  installedVersion?: string;
  categories: Category[];
  colors: {
    color1: string;
    color2: string;
  };
  requires_steamodded: boolean;
  requires_talisman: boolean;
  publisher: string;
  repo: string;
  downloadURL: string;
  folderName?: string | null;
  version?: string | null;
  installed: boolean;
  last_updated: number;
}

export interface LocalMod {
  name: string;
  id: string;
  author: string[];
  description: string;
  prefix: string;
  version?: string;
  path: string;
  dependencies: string[];
  conflicts: string[];
  is_tracked: boolean;
}

export enum SortOption {
  NameAsc = "name_asc",
  NameDesc = "name_desc",
  LastUpdatedAsc = "updated_asc",
  LastUpdatedDesc = "updated_desc",
}

export const backgroundEnabled = writable(false);

export const currentSort = writable<SortOption>(SortOption.LastUpdatedDesc);

/** Search/filter query for Installed Mods view */
export const installedModsSearchStore = writable<string>("");

export const updateAvailableStore = writable<{ [key: string]: boolean }>({});

export const modEnabledStore = writable<Record<string, boolean>>({});

// Multi-select state for bulk operations in Installed Mods view
export const selectedModsStore = writable<Set<string>>(new Set());

export function toggleModSelection(id: string): void {
  selectedModsStore.update((set) => {
    const newSet = new Set(set);
    if (newSet.has(id)) {
      newSet.delete(id);
    } else {
      newSet.add(id);
    }
    return newSet;
  });
}

export function selectAllMods(ids: string[]): void {
  selectedModsStore.set(new Set(ids));
}

export function clearSelection(): void {
  selectedModsStore.set(new Set());
}

// Batch update helper for modEnabledStore - single store update, single re-render
export function batchUpdateModEnabled(updates: Record<string, boolean>): void {
  modEnabledStore.update((store) => ({ ...store, ...updates }));
}

export interface UninstallDialogState {
  show: boolean;
  modName: string;
  modPath: string;
  dependents: string[];
}

export const uninstallDialogStore = writable<UninstallDialogState>({
  show: false,
  modName: "",
  modPath: "",
  dependents: [],
});

export const selectedModStore = writable<{ name: string; path: string } | null>(
  null,
);
export const dependentsStore = writable<string[]>([]);
export const currentPage = writable(1);
export const itemsPerPage = writable(12);
export const paginationWindow = writable({
  startPage: 1,
  totalPages: 1,
  maxVisiblePages: 5,
});

export type UninstallResult = {
  success: boolean;
  action: "cascade" | "force" | "single";
};

function readVersionCache(key: string): string[] {
  if (typeof window === "undefined") return [];
  try {
    return JSON.parse(localStorage.getItem(key) || "[]");
  } catch {
    return [];
  }
}

export const cachedVersions = writable<{
  steamodded: string[];
  talisman: string[];
}>({
  steamodded: readVersionCache("version-cache-steamodded"),
  talisman: readVersionCache("version-cache-talisman"),
});

if (typeof window !== "undefined") {
  cachedVersions.subscribe((value) => {
    try {
      localStorage.setItem(
        "version-cache-steamodded",
        JSON.stringify(value.steamodded),
      );
      localStorage.setItem(
        "version-cache-talisman",
        JSON.stringify(value.talisman),
      );
    } catch (_) {
      // Ignore storage quota errors; caching is optional.
    }
  });
}

export interface DependencyCheck {
  steamodded: boolean;
  talisman: boolean;
}

export interface InstalledMod {
  name: string;
  path: string;
  /** True when the remote index no longer lists this mod. */
  orphaned?: boolean;
  current_version?: string | null;
  dependencies?: string[];
  // collection_hash: string | null;
}

interface InstallationStatus {
  [key: string]: boolean;
}

export enum Category {
  Content = 0,
  Joker = 1,
  QualityOfLife = 2,
  Technical = 3,
  Miscellaneous = 4,
  ResourcePacks = 5,
  API = 6,
}

export const currentModView = writable<Mod | null>(null);
export const currentJokerView = writable<Mod | null>(null);
export const searchResults = writable<Mod[]>([]);
export const modsStore = writable<Mod[]>([]);

let persistSuspended = false;
let persistPending = false;
export async function withModsCachePersistenceSuspended<T>(
  fn: () => Promise<T>,
) {
  persistSuspended = true;
  try {
    return await fn();
  } finally {
    persistSuspended = false;
    if (persistPending) {
      persistPending = false;
      try {
        const current = get(modsStore);
        persistModsCache(current);
      } catch (_) {
        // ignore
      }
    }
  }
}

function persistModsCache(value: Mod[]) {
  try {
    const slim = value.map((m) => ({
      title: m.title,
      categories: m.categories,
      colors: m.colors,
      requires_steamodded: m.requires_steamodded,
      requires_talisman: m.requires_talisman,
      publisher: m.publisher,
      repo: m.repo,
      downloadURL: m.downloadURL,
      folderName: m.folderName ?? null,
      version: m.version ?? null,
      installed: m.installed,
      last_updated: m.last_updated,
      _dirName: m._dirName,
      _installedPath: m._installedPath,
      _hasThumbnail: m._hasThumbnail ?? true,
      // omit description and image fields (largest strings)
    }));

    const jsonData = JSON.stringify(slim);
    if (!safeLocalStorageSet("mods-cache", jsonData)) {
      // Quota exceeded - clear the cache entirely rather than corrupt it
      console.warn(
        "Failed to persist mods cache due to quota. Clearing cache.",
      );
      try {
        localStorage.removeItem("mods-cache");
        localStorage.removeItem("mods-cache-ts");
      } catch {
        // Ignore
      }
      return;
    }
    const now = Date.now();
    safeLocalStorageSet("mods-cache-ts", String(now));
    catalogLastRefreshed.set(now);
  } catch {
    try {
      localStorage.removeItem("mods-cache");
      localStorage.removeItem("mods-cache-ts");
    } catch {
      // ignore
    }
  }
}

// Background catalog loading state and last refresh time
export const catalogLoading = writable(false);
export const catalogLastRefreshed = writable<number | null>(null);
export const catalogResetNonce = writable(0);

// Persist and hydrate the mods catalog for instant UI + offline fallback
if (typeof window !== "undefined") {
  // Monitor storage quota on startup
  try {
    monitorStorageQuota();
  } catch {
    // Ignore monitoring errors
  }

  try {
    ensureCatalogSchema();
    const cached = localStorage.getItem("mods-cache");
    if (cached) {
      const parsed: Mod[] = JSON.parse(cached);
      if (Array.isArray(parsed)) {
        modsStore.set(parsed);
      }
    }
    const ts = localStorage.getItem("mods-cache-ts");
    if (ts) {
      const n = Number(ts);
      if (!Number.isNaN(n)) catalogLastRefreshed.set(n);
    }
  } catch {
    // ignore cache read errors
  }

  let persistTimer: number | null = null;
  modsStore.subscribe((value) => {
    if (persistTimer) {
      clearTimeout(persistTimer);
    }
    // Debounce cache writes to avoid thrashing localStorage during hydration.
    // Using 2000ms debounce and requestIdleCallback for better performance.
    persistTimer = window.setTimeout(() => {
      if (persistSuspended) {
        persistPending = true;
        return;
      }
      // Use requestIdleCallback when available for non-blocking persistence
      if ("requestIdleCallback" in window) {
        window.requestIdleCallback(
          () => persistModsCache(value),
          { timeout: 5000 }, // Ensure it runs within 5s even if browser is busy
        );
      } else {
        persistModsCache(value);
      }
    }, 2000);
  });
}

export const installationStatus: Writable<InstallationStatus> = writable({});

export const loadingStates2 = writable<{ [key: string]: boolean }>({});
//
//
// modsStore.subscribe(value => {
// 	if (typeof window !== 'undefined') {
// 		localStorage.setItem('mods', JSON.stringify(value));
// 	}
// });

function createPersistentCategory() {
  let storedCategory = "Popular";
  if (typeof window !== "undefined") {
    try {
      storedCategory = localStorage.getItem("currentCategory") || "Popular";
    } catch {
      // Ignore storage errors
    }
  }
  const { subscribe, set } = writable(storedCategory);

  return {
    subscribe,
    set: (value: string) => {
      try {
        localStorage.setItem("currentCategory", value);
      } catch (_) {
        // Ignore storage quota errors.
      }
      set(value);
    },
  };
}

export const currentCategory = createPersistentCategory();

export interface WarningPopupState {
  visible: boolean;
  message: string;
  onConfirm: () => void;
  onCancel: () => void;
}

export const showWarningPopup = writable<WarningPopupState>({
  visible: false,
  message: "",
  onConfirm: () => {},
  onCancel: () => {},
});

// Popup state to warn users when Lovely is not detected
export interface LovelyPopupState {
  visible: boolean;
  source?: "launch" | "other";
  onLaunchAnyway?: () => void | Promise<void>;
}

export const lovelyPopupStore = writable<LovelyPopupState>({
  visible: false,
});

// RequiresPopup state - shown when a mod has missing dependencies
export interface RequiresPopupState {
  visible: boolean;
  requiresSteamodded: boolean;
  requiresTalisman: boolean;
  onProceed: () => void;
  onDependencyClick: (dependency: string) => void;
}

export const requiresPopupStore = writable<RequiresPopupState>({
  visible: false,
  requiresSteamodded: false,
  requiresTalisman: false,
  onProceed: () => {},
  onDependencyClick: () => {},
});

// SecurityPopup state - shown on first launch to warn about mods
export interface SecurityPopupState {
  visible: boolean;
  onAcknowledge: () => void;
  onCancel: () => void;
}

export const securityPopupStore = writable<SecurityPopupState>({
  visible: false,
  onAcknowledge: () => {},
  onCancel: () => {},
});

// UpdateAvailablePopup state
export interface UpdatePopupState {
  visible: boolean;
  currentVersion: string;
  latestVersion: string;
  onClose: () => void;
  onDontShow: () => void;
}

export const updatePopupStore = writable<UpdatePopupState>({
  visible: false,
  currentVersion: "",
  latestVersion: "",
  onClose: () => {},
  onDontShow: () => {},
});

// ReportIssue popup state
export interface ReportIssueState {
  visible: boolean;
}

export const reportIssueStore = writable<ReportIssueState>({
  visible: false,
});

// CreateBackup popup state
export interface CreateBackupPopupState {
  visible: boolean;
}

export const createBackupPopupStore = writable<CreateBackupPopupState>({
  visible: false,
});

// Backup restore confirmation popup state
export interface RestoreBackupPopupState {
  visible: boolean;
  backupId: string;
  backupName: string;
}

export const restoreBackupPopupStore = writable<RestoreBackupPopupState>({
  visible: false,
  backupId: "",
  backupName: "",
});

// Backup delete confirmation popup state
export interface DeleteBackupPopupState {
  visible: boolean;
  backupId: string;
  backupName: string;
}

export const deleteBackupPopupStore = writable<DeleteBackupPopupState>({
  visible: false,
  backupId: "",
  backupName: "",
});
