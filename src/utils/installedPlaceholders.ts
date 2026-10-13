import type { InstalledMod, Mod, MultiplayerCard } from "../stores/modStore";
import { normalizeColorPair } from "./cardPalette";
import type { GameModeInfo, MultiplayerStatus, TrackInfo } from "./multiplayer";

export const normalizeModName = (name: string) =>
  name.toLowerCase().replace(/[^a-z0-9]/g, "");

export const ORPHAN_DESCRIPTION =
  "Installed locally but no longer listed in the mod index";

export const MULTIPLAYER_NAMESPACE = "BalatroMultiplayer";
export const MULTIPLAYER_DESCRIPTION =
  "Official Balatro Multiplayer package, managed from the Multiplayer tab";

const AUTO_DESCRIPTIONS = new Set([
  ORPHAN_DESCRIPTION,
  MULTIPLAYER_DESCRIPTION,
]);
const COVER_IMAGE = "/images/cover.jpg";

/** A card built from a DB row for an installed mod the catalog does not list */
export function isInstalledPlaceholder(mod: Mod): boolean {
  return !mod._dirName && Boolean(mod._installedPath);
}

/** Whether a mod belongs in All Mods, category tabs and search */
export function isBrowsableMod(mod: Mod): boolean {
  return !mod.orphaned && !mod.multiplayer && !isInstalledPlaceholder(mod);
}

const MULTIPLAYER_PREFIX = `${MULTIPLAYER_NAMESPACE.toLowerCase()}-`;
const LEGACY_PACKAGE = `${MULTIPLAYER_PREFIX}multiplayer`;
const API_PACKAGE = `${MULTIPLAYER_PREFIX}multiplayerapi`;

/** The Multiplayer tab entry tracking this row, matched by package id or tracked name */
function multiplayerEntry(
  row: InstalledMod,
  status: MultiplayerStatus | null,
): { role: MultiplayerCard["role"]; info: TrackInfo | GameModeInfo } | null {
  if (!status) return null;
  const id = thunderstoreId(row);
  for (const info of status.tracks) {
    if (info.tracked_name === row.name || lower(info.full_name) === id) {
      return { role: info.track, info };
    }
  }
  for (const info of status.gamemodes) {
    if (
      info.tracked_name === row.name ||
      (info.full_name !== null && lower(info.full_name) === id)
    ) {
      return { role: "gamemode", info };
    }
  }
  return null;
}

/**
 * The Multiplayer card info of a row installed from the BalatroMultiplayer namespace,
 * enriched from the Multiplayer tab status when it is known. Null for any other row
 */
export function multiplayerCardFor(
  row: InstalledMod,
  status: MultiplayerStatus | null = null,
): MultiplayerCard | null {
  const entry = multiplayerEntry(row, status);
  const id = thunderstoreId(row);
  if (entry) {
    return {
      role: entry.role,
      fullName: entry.info.full_name ?? row.source_id ?? null,
      latestVersion: entry.info.latest_version,
      updateAvailable: entry.info.update_available,
    };
  }
  if (!id?.startsWith(MULTIPLAYER_PREFIX)) return null;
  return {
    role:
      id === LEGACY_PACKAGE
        ? "legacy"
        : id === API_PACKAGE
          ? "api"
          : "gamemode",
    fullName: row.source_id ?? null,
    latestVersion: null,
    updateAvailable: false,
  };
}

/** Whether a row may belong to the Multiplayer tab, worth asking it for its status */
export function mayBeMultiplayerRow(row: InstalledMod): boolean {
  if (multiplayerCardFor(row)) return true;
  const name = normalizeModName(row.name);
  return name === "multiplayer" || name === "multiplayerapi";
}

/** Thunderstore namespace of a row, the package owner */
function ownerOf(row: InstalledMod): string | null {
  if (row.source !== "thunderstore" || !row.source_id) return null;
  const dash = row.source_id.indexOf("-");
  return dash > 0 ? row.source_id.slice(0, dash) : null;
}

function createPlaceholder(
  row: InstalledMod,
  status: MultiplayerStatus | null = null,
): Mod {
  const multiplayer = multiplayerCardFor(row, status);
  const info = multiplayerEntry(row, status)?.info;
  const orphaned = !multiplayer && row.orphaned === true;
  const icon = info?.icon_url || undefined;
  return {
    title: row.name,
    description: multiplayer
      ? info?.description || MULTIPLAYER_DESCRIPTION
      : orphaned
        ? ORPHAN_DESCRIPTION
        : "",
    image: COVER_IMAGE,
    thumbnailUrl: icon,
    colors: normalizeColorPair(null),
    categories: [],
    requires_steamodded: row.dependencies?.includes("Steamodded") ?? false,
    requires_talisman: row.dependencies?.includes("Talisman") ?? false,
    publisher: multiplayer
      ? MULTIPLAYER_NAMESPACE
      : (ownerOf(row) ?? "Installed"),
    repo: "",
    downloadURL: "",
    folderName: row.name,
    version: null,
    installedVersion: row.current_version || undefined,
    installed: true,
    last_updated: 0,
    _hasThumbnail: Boolean(icon),
    _installedPath: row.path,
    orphaned,
    ...(multiplayer ? { multiplayer, deprecated: info?.deprecated } : {}),
  };
}

const sameCard = (a?: MultiplayerCard, b?: MultiplayerCard) =>
  JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

/**
 * Refreshes the fields a placeholder derives from its row, fields filled from elsewhere
 * (a cached thumbnail, the description of a catalog card it replaced) are kept
 */
function syncPlaceholder(
  mod: Mod,
  row: InstalledMod,
  status: MultiplayerStatus | null,
): Mod {
  const fresh = createPlaceholder(row, status);
  const multiplayer = fresh.multiplayer;
  const description =
    !mod.description || AUTO_DESCRIPTIONS.has(mod.description)
      ? fresh.description
      : mod.description;
  const image = mod.image || fresh.image;
  const thumbnailUrl = multiplayer
    ? (fresh.thumbnailUrl ?? mod.thumbnailUrl)
    : mod.thumbnailUrl;
  const hasThumbnail = multiplayer
    ? Boolean(thumbnailUrl)
    : (mod._hasThumbnail ?? false);
  const publisher =
    multiplayer || !mod.publisher ? fresh.publisher : mod.publisher;
  const deprecated = multiplayer ? fresh.deprecated : mod.deprecated;
  if (
    Boolean(mod.orphaned) === Boolean(fresh.orphaned) &&
    mod.installedVersion === fresh.installedVersion &&
    mod._installedPath === row.path &&
    mod.description === description &&
    mod.image === image &&
    mod.thumbnailUrl === thumbnailUrl &&
    mod._hasThumbnail === hasThumbnail &&
    mod.publisher === publisher &&
    mod.deprecated === deprecated &&
    sameCard(mod.multiplayer, multiplayer)
  ) {
    return mod;
  }
  const next: Mod = {
    ...mod,
    orphaned: fresh.orphaned,
    installedVersion: fresh.installedVersion,
    _installedPath: row.path,
    description,
    image,
    thumbnailUrl,
    _hasThumbnail: hasThumbnail,
    publisher,
    deprecated,
  };
  if (multiplayer) next.multiplayer = multiplayer;
  else delete next.multiplayer;
  return next;
}

const lower = (s: string) => s.toLowerCase();

/** A row installed from Thunderstore, only the card of that package owns it */
function thunderstoreId(row: InstalledMod): string | null {
  return row.source === "thunderstore" && row.source_id
    ? lower(row.source_id)
    : null;
}

export interface InstalledRowIndex {
  byExact: Map<string, InstalledMod>;
  byNormalized: Map<string, InstalledMod>;
  bySourceId: Map<string, InstalledMod>;
  /** Thunderstore rows by exact name, for the placeholder cards built from them */
  thunderstoreByName: Map<string, InstalledMod>;
}

export function indexInstalledRows(
  installed: InstalledMod[],
): InstalledRowIndex {
  const index: InstalledRowIndex = {
    byExact: new Map(),
    byNormalized: new Map(),
    bySourceId: new Map(),
    thunderstoreByName: new Map(),
  };
  for (const row of installed) {
    const id = thunderstoreId(row);
    if (id) {
      index.bySourceId.set(id, row);
      index.thunderstoreByName.set(row.name, row);
    } else {
      index.byExact.set(lower(row.name), row);
      index.byNormalized.set(normalizeModName(row.name), row);
    }
  }
  return index;
}

/**
 * The DB row tracking a card. Thunderstore rows match by package id so a package and
 * an index mod sharing a name never claim each other, index rows match by name
 */
export function findInstalledRow(
  mod: Mod,
  index: InstalledRowIndex,
): InstalledMod | null {
  const fullName = mod.thunderstore?.full_name;
  if (fullName) {
    const row = index.bySourceId.get(lower(fullName));
    if (row) return row;
  }
  if (isInstalledPlaceholder(mod)) {
    const row = index.thunderstoreByName.get(mod.title);
    if (row) return row;
  }
  return (
    index.byExact.get(lower(mod.title)) ??
    index.byNormalized.get(normalizeModName(mod.title)) ??
    null
  );
}

/** Installed flag of every card, keyed by title like `installationStatus` */
export function computeInstallationStatus(
  mods: Mod[],
  installed: InstalledMod[],
): Record<string, boolean> {
  const index = indexInstalledRows(installed);
  const out: Record<string, boolean> = {};
  for (const mod of mods) {
    out[mod.title] = out[mod.title] || Boolean(findInstalledRow(mod, index));
  }
  return out;
}

/**
 * Keeps one card per installed DB row in the mods list. Rows the catalog lists
 * reuse the catalog card (with the orphan flag synced), every other row gets a
 * placeholder card, and placeholders for rows that are gone or now listed are
 * dropped. Returns the input array when nothing changed
 */
export function reconcileInstalledPlaceholders(
  mods: Mod[],
  installed: InstalledMod[],
  status: MultiplayerStatus | null = null,
): Mod[] {
  const index = indexInstalledRows(installed);

  const catalogExact = new Set<string>();
  const catalogNormalized = new Set<string>();
  const catalogPackages = new Set<string>();
  for (const m of mods) {
    if (isInstalledPlaceholder(m)) continue;
    catalogExact.add(lower(m.title));
    catalogNormalized.add(normalizeModName(m.title));
    if (m.thunderstore?.full_name) {
      catalogPackages.add(lower(m.thunderstore.full_name));
    }
  }
  const nameInCatalog = (name: string) =>
    catalogExact.has(lower(name)) ||
    catalogNormalized.has(normalizeModName(name));
  const rowInCatalog = (row: InstalledMod) => {
    // Multiplayer packages are never in the general catalog, they have their own tab
    if (multiplayerCardFor(row, status)) return false;
    const id = thunderstoreId(row);
    return (id !== null && catalogPackages.has(id)) || nameInCatalog(row.name);
  };

  const installedByName = new Map(installed.map((r) => [r.name, r]));
  const placed = new Set<string>();
  let changed = false;
  const out: Mod[] = [];

  for (const m of mods) {
    if (isInstalledPlaceholder(m)) {
      const row = installedByName.get(m.title);
      if (!row || rowInCatalog(row) || placed.has(m.title)) {
        changed = true;
        continue;
      }
      placed.add(m.title);
      const next = syncPlaceholder(m, row, status);
      if (next !== m) changed = true;
      out.push(next);
      continue;
    }

    const row = findInstalledRow(m, index);
    // A mod the index dropped but Thunderstore still lists keeps a live card
    const orphaned = row?.orphaned === true && m.source !== "thunderstore";
    const installedVersion = orphaned
      ? row?.current_version || undefined
      : undefined;
    if (
      Boolean(m.orphaned) !== orphaned ||
      m.installedVersion !== installedVersion
    ) {
      changed = true;
      out.push({ ...m, orphaned, installedVersion });
    } else {
      out.push(m);
    }
  }

  for (const row of installed) {
    if (placed.has(row.name) || rowInCatalog(row)) continue;
    placed.add(row.name);
    out.push(createPlaceholder(row, status));
    changed = true;
  }

  return changed ? out : mods;
}

/** True when an update is pending that Update All can install */
export function hasActionableUpdate(
  mods: Mod[],
  updates: Record<string, boolean>,
): boolean {
  // Multiplayer packages update from the Multiplayer tab, Update All skips them
  const orphaned = new Set(
    mods.filter((m) => m.orphaned || m.multiplayer).map((m) => m.title),
  );
  return Object.entries(updates).some(
    ([title, available]) => available === true && !orphaned.has(title),
  );
}
