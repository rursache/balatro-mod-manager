import type { InstalledMod, Mod } from "../stores/modStore";
import { normalizeColorPair } from "./cardPalette";

export const normalizeModName = (name: string) =>
  name.toLowerCase().replace(/[^a-z0-9]/g, "");

export const ORPHAN_DESCRIPTION =
  "Installed locally but no longer listed in the mod index";

/** A card built from a DB row for an installed mod the catalog does not list */
export function isInstalledPlaceholder(mod: Mod): boolean {
  return !mod._dirName && Boolean(mod._installedPath);
}

/** Whether a mod belongs in All Mods, category tabs and search */
export function isBrowsableMod(mod: Mod): boolean {
  return !mod.orphaned && !isInstalledPlaceholder(mod);
}

function createPlaceholder(row: InstalledMod): Mod {
  const orphaned = row.orphaned === true;
  return {
    title: row.name,
    description: orphaned ? ORPHAN_DESCRIPTION : "",
    image: "/images/cover.jpg",
    colors: normalizeColorPair(null),
    categories: [],
    requires_steamodded: row.dependencies?.includes("Steamodded") ?? false,
    requires_talisman: row.dependencies?.includes("Talisman") ?? false,
    publisher: "Installed",
    repo: "",
    downloadURL: "",
    folderName: row.name,
    version: null,
    installedVersion: row.current_version || undefined,
    installed: true,
    last_updated: 0,
    _hasThumbnail: false,
    _installedPath: row.path,
    orphaned,
  };
}

function syncPlaceholder(mod: Mod, row: InstalledMod): Mod {
  const orphaned = row.orphaned === true;
  const installedVersion = row.current_version || undefined;
  const autoDescription =
    mod.description === "" || mod.description === ORPHAN_DESCRIPTION;
  const description = autoDescription
    ? orphaned
      ? ORPHAN_DESCRIPTION
      : ""
    : mod.description;
  if (
    Boolean(mod.orphaned) === orphaned &&
    mod.installedVersion === installedVersion &&
    mod._installedPath === row.path &&
    mod.description === description
  ) {
    return mod;
  }
  return {
    ...mod,
    orphaned,
    installedVersion,
    _installedPath: row.path,
    description,
  };
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
      const next = syncPlaceholder(m, row);
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
    out.push(createPlaceholder(row));
    changed = true;
  }

  return changed ? out : mods;
}

/** True when an update is pending for a mod the index still lists */
export function hasActionableUpdate(
  mods: Mod[],
  updates: Record<string, boolean>,
): boolean {
  const orphaned = new Set(mods.filter((m) => m.orphaned).map((m) => m.title));
  return Object.entries(updates).some(
    ([title, available]) => available === true && !orphaned.has(title),
  );
}
