// stores/modCache.ts
import { writable, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { InstalledMod } from "./modStore";
import { installationStatus, modsStore } from "./modStore";
import {
  computeInstallationStatus,
  mayBeMultiplayerRow,
  reconcileInstalledPlaceholders,
} from "../utils/installedPlaceholders";
import { multiplayerStatusStore } from "./multiplayerStatus";
import type { MultiplayerStatus } from "../utils/multiplayer";

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

function updateInstallationStatus(installed: InstalledMod[]) {
  installationStatus.set(computeInstallationStatus(get(modsStore), installed));
}

/**
 * Mirrors installed DB rows into the mods list so mods missing from the
 * catalog (orphaned or not loaded yet) still get a card in Installed Mods
 */
export function syncInstalledPlaceholders(installed: InstalledMod[]) {
  const before = get(modsStore);
  const after = reconcileInstalledPlaceholders(
    before,
    installed,
    get(multiplayerStatusStore),
  );
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
              source: mod.source ?? "index",
              source_id: mod.source_id ?? null,
            }));

            cache.set(formattedMods);
            syncInstalledPlaceholders(formattedMods);
            lastFetchTime = Date.now();
            if (!get(multiplayerStatusStore)) {
              void refreshMultiplayerStatus(formattedMods);
            }
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

let multiplayerInFlight: Promise<MultiplayerStatus | null> | null = null;

/**
 * Loads the Multiplayer tab status when a Multiplayer package is installed, so its
 * Installed card gets the package icon, description and update state of that tab
 */
export function refreshMultiplayerStatus(
  installed: InstalledMod[] = get(modCache.installedModsCache),
): Promise<MultiplayerStatus | null> {
  if (!installed.some(mayBeMultiplayerRow)) {
    return Promise.resolve(get(multiplayerStatusStore));
  }
  if (!multiplayerInFlight) {
    multiplayerInFlight = invoke<MultiplayerStatus>("multiplayer_status")
      .then((status) => {
        multiplayerStatusStore.set(status);
        return status;
      })
      .catch((error) => {
        console.warn("Failed to load the Multiplayer status:", error);
        return get(multiplayerStatusStore);
      })
      .finally(() => {
        multiplayerInFlight = null;
      });
  }
  return multiplayerInFlight;
}

// A new status (from here or the Multiplayer tab) refreshes the Multiplayer cards
let lastStatus: MultiplayerStatus | null = null;
multiplayerStatusStore.subscribe((status) => {
  if (status === lastStatus) return;
  lastStatus = status;
  const installed = get(modCache.installedModsCache);
  if (installed.length > 0) syncInstalledPlaceholders(installed);
});

// Listen for backend notifications that installed mods have changed,
// and refresh the cache immediately to update the UI in real-time.
// Guard against duplicate listeners during Vite HMR by stashing a flag on window.
try {
  if (typeof window !== "undefined") {
    if (!window.__bmmInstalledModsListenerAttached) {
      window.__bmmInstalledModsListenerAttached = true;
      listen<ModsChangedEvent>("installed-mods-changed", async () => {
        try {
          // A refresh recomputes every installed flag, rows from Thunderstore are
          // matched by package id so name deltas alone are not enough
          await modCache.forceRefreshCache();
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
