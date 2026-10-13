import type { Mod } from "../stores/modStore";
import type {
  CatalogStatus,
  SourceFilter,
  SourceStatus,
} from "../stores/catalogFilters";

export interface CatalogFilterOptions {
  thunderstoreEnabled: boolean;
  showNsfw: boolean;
  hideAiGenerated: boolean;
  showDeprecated: boolean;
  source: SourceFilter;
}

export const DEFAULT_CATALOG_FILTERS: CatalogFilterOptions = {
  thunderstoreEnabled: true,
  showNsfw: false,
  hideAiGenerated: false,
  showDeprecated: false,
  source: "all",
};

export function isThunderstoreOnly(mod: Mod): boolean {
  return mod.source === "thunderstore";
}

export function listedOnThunderstore(mod: Mod): boolean {
  return mod.source === "thunderstore" || mod.source === "merged";
}

export function listedOnIndex(mod: Mod): boolean {
  return mod.source === undefined || mod.source !== "thunderstore";
}

/**
 * Whether a catalog card shows while browsing. Installed Mods skips these filters so
 * deprecated or hidden packages a user already has stay visible there
 */
export function passesCatalogFilters(
  mod: Mod,
  options: CatalogFilterOptions,
): boolean {
  if (!options.thunderstoreEnabled && isThunderstoreOnly(mod)) return false;
  if (mod.deprecated && !options.showDeprecated) return false;
  if (mod.nsfw && !options.showNsfw) return false;
  if (mod.aiGenerated && options.hideAiGenerated) return false;
  switch (options.source) {
    case "index":
      return listedOnIndex(mod);
    case "thunderstore":
      return options.thunderstoreEnabled && listedOnThunderstore(mod);
    default:
      return true;
  }
}

function formatAge(seconds: number): string {
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

function describeSource(
  label: string,
  status: SourceStatus,
  nowSeconds: number,
): string {
  const synced = status.synced_at > 0;
  const age = synced
    ? formatAge(Math.max(0, nowSeconds - status.synced_at))
    : "";
  if (status.ok) return synced ? `${label} synced ${age}` : `${label} synced`;
  if (synced) return `${label} offline, cached ${age}`;
  return `${label} unavailable`;
}

/** One line summary such as "Index synced 5m ago, Thunderstore offline, cached 2h ago" */
export function formatSyncStatus(
  status: CatalogStatus | null,
  nowMs: number,
  thunderstoreEnabled = true,
): string {
  if (!status) return "";
  const now = Math.floor(nowMs / 1000);
  const parts = [describeSource("Index", status.index, now)];
  if (thunderstoreEnabled) {
    parts.push(describeSource("Thunderstore", status.thunderstore, now));
  }
  return parts.join(", ");
}

/** True when a source failed, the line is then highlighted */
export function syncHasProblem(
  status: CatalogStatus | null,
  thunderstoreEnabled = true,
): boolean {
  if (!status) return false;
  return !status.index.ok || (thunderstoreEnabled && !status.thunderstore.ok);
}

const COMPACT = new Intl.NumberFormat("en", {
  notation: "compact",
  maximumFractionDigits: 1,
});

/** 1234 -> "1.2K", null for unknown counts */
export function formatDownloads(value?: number | null) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) {
    return null;
  }
  return COMPACT.format(value);
}
