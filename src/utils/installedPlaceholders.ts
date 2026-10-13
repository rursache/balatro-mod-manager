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
  const rowsByExact = new Map<string, InstalledMod>();
  const rowsByNormalized = new Map<string, InstalledMod>();
  for (const row of installed) {
    rowsByExact.set(row.name.toLowerCase(), row);
    rowsByNormalized.set(normalizeModName(row.name), row);
  }
  const findRow = (title: string) =>
    rowsByExact.get(title.toLowerCase()) ??
    rowsByNormalized.get(normalizeModName(title));

  const catalogExact = new Set<string>();
  const catalogNormalized = new Set<string>();
  for (const m of mods) {
    if (isInstalledPlaceholder(m)) continue;
    catalogExact.add(m.title.toLowerCase());
    catalogNormalized.add(normalizeModName(m.title));
  }
  const inCatalog = (name: string) =>
    catalogExact.has(name.toLowerCase()) ||
    catalogNormalized.has(normalizeModName(name));

  const installedByName = new Map(installed.map((r) => [r.name, r]));
  const placed = new Set<string>();
  let changed = false;
  const out: Mod[] = [];

  for (const m of mods) {
    if (isInstalledPlaceholder(m)) {
      const row = installedByName.get(m.title);
      if (!row || inCatalog(m.title) || placed.has(m.title)) {
        changed = true;
        continue;
      }
      placed.add(m.title);
      const next = syncPlaceholder(m, row);
      if (next !== m) changed = true;
      out.push(next);
      continue;
    }

    const row = findRow(m.title);
    const orphaned = row?.orphaned === true;
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
    if (placed.has(row.name) || inCatalog(row.name)) continue;
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
