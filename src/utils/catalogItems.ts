import type {
  CatalogSource,
  InstallSourceName,
  Mod,
  ThunderstoreInfo,
} from "../stores/modStore";

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
