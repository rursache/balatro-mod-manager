// stores/modCache.ts
import { writable, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { InstalledMod } from "./modStore";
import { installationStatus, modsStore } from "./modStore";
import {
  normalizeModName,
  reconcileInstalledPlaceholders,
} from "../utils/installedPlaceholders";

declare global {
  // eslint-disable-next-line no-unused-vars
  interface Window {
    __bmmInstalledModsListenerAttached?: boolean;
    __bmmInstalledModsUnlisten?: () => void;
  }
}

/** Event payload from backend with delta information */
interface ModsChangedEvent {
  added: string[];
  removed: string[];
  full_refresh: boolean;
}

// Cached normalized sets to avoid rebuilding on every change
let cachedInstalledNormalized: Set<string> = new Set();
let cachedInstalledExact: Set<string> = new Set();
let lastInstalledModsHash = "";

// Compute a simple hash of installed mods for change detection
function computeInstalledHash(mods: InstalledMod[]): string {
  return mods
    .map((m) => m.name)
    .sort()
    .join("|");
}

// Update installation status incrementally when possible
function updateInstallationStatus(installed: InstalledMod[]) {
  const newHash = computeInstalledHash(installed);

  // Only rebuild the sets if the installed mods actually changed
  if (newHash !== lastInstalledModsHash) {
    cachedInstalledNormalized = new Set(
      installed.map((mod) => normalizeModName(mod.name)),
    );
    cachedInstalledExact = new Set(
      installed.map((mod) => mod.name.toLowerCase()),
    );
    lastInstalledModsHash = newHash;
  }

  const mods = get(modsStore);

  // Build the status map using cached sets
  const newStatus: { [key: string]: boolean } = {};
  for (const mod of mods) {
    newStatus[mod.title] =
      cachedInstalledExact.has(mod.title.toLowerCase()) ||
      cachedInstalledNormalized.has(normalizeModName(mod.title));
  }

  installationStatus.set(newStatus);
}

// Update installation status incrementally using delta information
function updateInstallationStatusDelta(added: string[], removed: string[]) {
  const currentStatus = get(installationStatus);
  const newStatus = { ...currentStatus };
  const mods = get(modsStore);

  // Update cached sets
  for (const name of added) {
    cachedInstalledExact.add(name.toLowerCase());
    cachedInstalledNormalized.add(normalizeModName(name));
  }
  for (const name of removed) {
    cachedInstalledExact.delete(name.toLowerCase());
    cachedInstalledNormalized.delete(normalizeModName(name));
  }

  // Only update status for affected mods
  for (const mod of mods) {
    const titleLower = mod.title.toLowerCase();
    const titleNormalized = normalizeModName(mod.title);

    // Check if this mod was in the added/removed lists
    const wasAffected =
      added.some(
        (n) =>
          n.toLowerCase() === titleLower ||
          normalizeModName(n) === titleNormalized,
      ) ||
      removed.some(
        (n) =>
          n.toLowerCase() === titleLower ||
          normalizeModName(n) === titleNormalized,
      );

    if (wasAffected) {
      newStatus[mod.title] =
        cachedInstalledExact.has(titleLower) ||
        cachedInstalledNormalized.has(titleNormalized);
    }
  }

  installationStatus.set(newStatus);
}

/**
 * Mirrors installed DB rows into the mods list so mods missing from the
 * catalog (orphaned or not loaded yet) still get a card in Installed Mods
 */
export function syncInstalledPlaceholders(installed: InstalledMod[]) {
  const before = get(modsStore);
  const after = reconcileInstalledPlaceholders(before, installed);
  if (after !== before) modsStore.set(after);
  updateInstallationStatus(installed);
}

// Create a self-contained cache system
const createModCache = () => {
  // Private variables inside closure
  const cache = writable<InstalledMod[]>([]);
  let lastFetchTime = 0;
  const CACHE_TIMEOUT = 15000; // 15 seconds to avoid chatty IPC
  let inFlight: Promise<InstalledMod[]> | null = null; // coalesce concurrent calls

  // Core cache function that handles all operations
  async function getModsFromCache(
    forceRefresh = false,
  ): Promise<InstalledMod[]> {
    const now = Date.now();

    if (
      forceRefresh ||
      lastFetchTime === 0 ||
      now - lastFetchTime > CACHE_TIMEOUT
    ) {
      try {
        // Deduplicate concurrent requests
        if (!inFlight) {
          inFlight = (async () => {
            const installed: InstalledMod[] = await invoke(
              "get_installed_mods_from_db",
            );
            const formattedMods = installed.map((mod) => ({
              name: mod.name,
              path: mod.path,
              orphaned: mod.orphaned === true,
              current_version: mod.current_version ?? null,
              dependencies: mod.dependencies ?? [],
            }));

            cache.set(formattedMods);
            syncInstalledPlaceholders(formattedMods);
            lastFetchTime = Date.now();
            return formattedMods;
          })();
        }

        const result = await inFlight;
        inFlight = null;
        return result;
      } catch (error) {
        console.error("Failed to get installed mods:", error);
        inFlight = null;
        return [];
      }
    }

    // Return current value from store
    return get(cache);
  }

  // Public interface
  return {
    // Exported store for reactive access
    installedModsCache: cache,

    // Get mods with optional force refresh
    fetchCachedMods: async (forceRefresh = false) => {
      return getModsFromCache(forceRefresh);
    },

    // Check if a specific mod is in the cache
    checkModInCache: async (modTitle: string) => {
      if (!modTitle) return false;
      const mods = await getModsFromCache();
      return mods.some((m) => m.name.toLowerCase() === modTitle.toLowerCase());
    },

    // Force refresh the cache
    forceRefreshCache: async () => {
      return getModsFromCache(true);
    },
  };
};

// Create a single instance of the cache system
const modCache = createModCache();

// Listen for backend notifications that installed mods have changed,
// and refresh the cache immediately to update the UI in real-time.
// Guard against duplicate listeners during Vite HMR by stashing a flag on window.
try {
  if (typeof window !== "undefined") {
    if (!window.__bmmInstalledModsListenerAttached) {
      window.__bmmInstalledModsListenerAttached = true;
      listen<ModsChangedEvent>("installed-mods-changed", async (event) => {
        try {
          const payload = event.payload;

          // If we have delta information and it's not a full refresh, use incremental update
          if (
            !payload.full_refresh &&
            (payload.added.length > 0 || payload.removed.length > 0)
          ) {
            // Update the cache for consistency
            await modCache.forceRefreshCache();
            // Use delta-based status update for efficiency
            updateInstallationStatusDelta(payload.added, payload.removed);
          } else {
            // Full refresh - fetch all and rebuild status
            const installed = await modCache.forceRefreshCache();
            updateInstallationStatus(installed);
          }
        } catch {
          // ignore
        }
      })
        .then((un) => {
          window.__bmmInstalledModsUnlisten = un;
          // Always tear down on full-page unload (webview reload, app exit).
          // HMR has its own dispose hook below for module replacement.
          const unloadHandler = () => {
            try {
              window.__bmmInstalledModsUnlisten?.();
            } catch (err) {
              console.warn("Failed to unlisten installed-mods on unload:", err);
            }
            window.__bmmInstalledModsListenerAttached = false;
            window.__bmmInstalledModsUnlisten = undefined;
          };
          try {
            window.addEventListener("beforeunload", unloadHandler, {
              once: true,
            });
          } catch (_) {
            // ignore — beforeunload unsupported in some embeddings
          }
          if (import.meta?.hot) {
            import.meta.hot.dispose(() => {
              try {
                window.removeEventListener("beforeunload", unloadHandler);
              } catch (_) {
                /* ignore */
              }
              unloadHandler();
            });
          }
        })
        .catch((err) => {
          console.warn("Failed to attach installed-mods listener:", err);
        });
    }
  }
} catch {
  // ignore if listen fails outside Tauri context
}

// Export the public interface
export const {
  installedModsCache,
  fetchCachedMods,
  checkModInCache,
  forceRefreshCache,
} = modCache;
