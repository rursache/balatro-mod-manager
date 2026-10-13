import { writable } from "svelte/store";
import { createPersistentBoolean } from "./ui";

/** Lists Thunderstore packages next to the index, off keeps merged mods on the index */
export const thunderstoreEnabled = createPersistentBoolean(
  "catalog.thunderstoreEnabled",
  true,
);
export const showNsfwMods = createPersistentBoolean("catalog.showNsfw", false);
export const hideAiGeneratedMods = createPersistentBoolean(
  "catalog.hideAiGenerated",
  false,
);
export const showDeprecatedMods = createPersistentBoolean(
  "catalog.showDeprecated",
  false,
);

export type SourceFilter = "all" | "index" | "thunderstore";

/** Source chips above the catalog grid */
export const sourceFilter = writable<SourceFilter>("all");

export interface SourceStatus {
  ok: boolean;
  /** Unix seconds, 0 when never synced */
  synced_at: number;
  error: string | null;
  count: number;
}

export interface CatalogStatus {
  index: SourceStatus;
  thunderstore: SourceStatus;
}

export const catalogStatusStore = writable<CatalogStatus | null>(null);
