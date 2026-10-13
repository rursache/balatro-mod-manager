import type {
  CatalogSource,
  InstallSourceName,
  Mod,
  ThunderstoreInfo,
} from "../stores/modStore";
import { normalizeModName } from "./installedPlaceholders";

export interface ArchiveModMeta {
  title: string;
  "requires-steamodded"?: boolean;
  "requires-talisman"?: boolean;
  requires_steamodded?: boolean;
  requires_talisman?: boolean;
  categories: string[];
  author: string;
  repo: string;
  downloadURL?: string | null;
  folderName?: string;
  version?: string;
  "last-updated"?: number;
}

/** One entry of `fetch_repo_mods` */
export interface ArchiveModItem {
  dir_name: string;
  meta: ArchiveModMeta;
  description: string;
  image_url: string;
  has_thumbnail?: boolean;
  source?: CatalogSource;
  thunderstore?: ThunderstoreInfo | null;
  downloads?: number | null;
}

export const TS_ID_PREFIX = "ts:";

export type CatalogSourceFields = Pick<
  Mod,
  | "source"
  | "sources"
  | "thunderstore"
  | "downloads"
  | "deprecated"
  | "nsfw"
  | "aiGenerated"
>;

export function catalogSourceFields(item: ArchiveModItem): CatalogSourceFields {
  const thunderstore = item.thunderstore ?? null;
  const source: CatalogSource =
    item.source ?? (thunderstore ? "merged" : "index");
  const sources: InstallSourceName[] =
    source === "merged"
      ? ["index", "thunderstore"]
      : source === "thunderstore"
        ? ["thunderstore"]
        : ["index"];
  const downloads =
    typeof item.downloads === "number"
      ? item.downloads
      : (thunderstore?.downloads_total ?? null);
  return {
    source,
    sources,
    thunderstore,
    downloads,
    deprecated: thunderstore?.deprecated === true,
    nsfw: thunderstore?.nsfw === true,
    aiGenerated: thunderstore?.ai_generated === true,
  };
}

/**
 * Card titles keyed by catalog id. Titles are the frontend's mod key, so a
 * Thunderstore only package sharing a name with another entry is shown as
 * "Title (Namespace)", the same name the backend gives its row on a collision
 */
export function uniqueCatalogTitles(
  items: readonly ArchiveModItem[],
): Map<string, string> {
  const groups = new Map<string, ArchiveModItem[]>();
  for (const item of items) {
    const key = normalizeModName(item.meta.title);
    const group = groups.get(key);
    if (group) group.push(item);
    else groups.set(key, [item]);
  }
  const out = new Map<string, string>();
  for (const group of groups.values()) {
    const owner =
      group.find((i) => (i.source ?? "index") !== "thunderstore") ??
      [...group].sort((a, b) => a.dir_name.localeCompare(b.dir_name))[0];
    for (const item of group) {
      const namespace = item.thunderstore?.namespace;
      const rename =
        item !== owner && item.source === "thunderstore" && Boolean(namespace);
      out.set(
        item.dir_name,
        rename ? `${item.meta.title} (${namespace})` : item.meta.title,
      );
    }
  }
  return out;
}

/** Catalog id for `plan_catalog_install`, also covers cards built before ids existed */
export function catalogInstallId(mod: Mod): string | null {
  return mod._dirName || mod.id || null;
}

export const STEAMODDED_ID = "Steamodded@smods";

/** Thunderstore backed cards install through the catalog planner, Steamodded keeps its own installer */
export function usesCatalogInstall(mod: Mod): boolean {
  const id = catalogInstallId(mod);
  return Boolean(
    mod.thunderstore &&
    (mod.source === "thunderstore" || mod.source === "merged") &&
    id &&
    id !== STEAMODDED_ID,
  );
}
