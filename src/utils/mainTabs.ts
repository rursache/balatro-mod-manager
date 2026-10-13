export type MainTab =
  | "installed"
  | "catalog"
  | "multiplayer"
  | "collections"
  | "backups"
  | "settings"
  | "about";

export const MAIN_TABS: { id: MainTab; label: string }[] = [
  { id: "installed", label: "Installed Mods" },
  { id: "catalog", label: "All Mods" },
  { id: "multiplayer", label: "Multiplayer" },
  { id: "collections", label: "Collections" },
  { id: "backups", label: "Backups" },
  { id: "settings", label: "Settings" },
  { id: "about", label: "About" },
];

export const INSTALLED_CATEGORY = "Installed Mods";
export const MULTIPLAYER_CATEGORY = "Multiplayer";
export const COLLECTIONS_CATEGORY = "Collections";
export const SEARCH_CATEGORY = "Search";
export const DEFAULT_CATALOG_CATEGORY = "All Mods";

/** Entries of the All Mods sidebar, in display order */
export const CATALOG_CATEGORIES = [
  SEARCH_CATEGORY,
  DEFAULT_CATALOG_CATEGORY,
  "Content",
  "Miscellaneous",
  "Joker",
  "Quality of Life",
  "Technical",
  "Resource Packs",
  "API",
] as const;

/** Tabs rendered by the Mods view, each one maps to a currentCategory value */
export type ModsTab = Extract<
  MainTab,
  "installed" | "catalog" | "multiplayer" | "collections"
>;

export function isModsTab(tab: MainTab): tab is ModsTab {
  return (
    tab === "installed" ||
    tab === "catalog" ||
    tab === "multiplayer" ||
    tab === "collections"
  );
}

export function tabForCategory(category: string): ModsTab {
  switch (category) {
    case INSTALLED_CATEGORY:
      return "installed";
    case MULTIPLAYER_CATEGORY:
      return "multiplayer";
    case COLLECTIONS_CATEGORY:
      return "collections";
    default:
      return "catalog";
  }
}

export function isCatalogCategory(category: string): boolean {
  return (CATALOG_CATEGORIES as readonly string[]).includes(category);
}

/** The category a Mods tab shows, the All Mods tab reopens the last catalog category */
export function categoryForTab(
  tab: ModsTab,
  lastCatalogCategory: string,
): string {
  switch (tab) {
    case "installed":
      return INSTALLED_CATEGORY;
    case "multiplayer":
      return MULTIPLAYER_CATEGORY;
    case "collections":
      return COLLECTIONS_CATEGORY;
    case "catalog":
      return isCatalogCategory(lastCatalogCategory)
        ? lastCatalogCategory
        : DEFAULT_CATALOG_CATEGORY;
  }
}

/** Only the paged catalog grid uses the pagination footer */
export function showsPagination(category: string): boolean {
  return isCatalogCategory(category) && category !== SEARCH_CATEGORY;
}

export function chooseStartupTab(
  trackedCount: number,
  localCount: number,
): ModsTab {
  return trackedCount + localCount > 0 ? "installed" : "catalog";
}

interface StartupSources {
  tracked: () => Promise<unknown[]>;
  local: () => Promise<unknown[]>;
  timeoutMs?: number;
}

async function countOf(load: () => Promise<unknown[]>): Promise<number> {
  try {
    const list = await load();
    return Array.isArray(list) ? list.length : 0;
  } catch {
    return 0;
  }
}

/**
 * Picks the startup tab once both installed lists are known, a hung IPC call
 * falls back to All Mods after the timeout
 */
export async function resolveStartupTab({
  tracked,
  local,
  timeoutMs = 4000,
}: StartupSources): Promise<ModsTab> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const timeout = new Promise<ModsTab>((resolve) => {
    timer = setTimeout(() => resolve("catalog"), timeoutMs);
  });
  const decided = (async () => {
    // A tracked mod is enough, skip the slower local scan
    const trackedCount = await countOf(tracked);
    if (trackedCount > 0) return chooseStartupTab(trackedCount, 0);
    return chooseStartupTab(0, await countOf(local));
  })();
  try {
    return await Promise.race([decided, timeout]);
  } finally {
    if (timer !== null) clearTimeout(timer);
  }
}
