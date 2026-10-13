import type { InstalledMod, Mod } from "../stores/modStore";
import {
  catalogInstallId,
  STEAMODDED_ID,
  usesCatalogInstall,
} from "./catalogItems";
import { isInstalledPlaceholder } from "./installedPlaceholders";

export type SpecialInstaller = "steamodded" | "talisman";
export type ReinstallKind = "thunderstore" | "index" | SpecialInstaller;

export interface ReinstallTarget {
  kind: ReinstallKind;
  row: InstalledMod;
  /** Version that gets installed, null when the index download decides */
  version: string | null;
  /** Index download URL, set for index reinstalls */
  url?: string;
}

/** Mods with their own installer, they never switch catalog source */
export function specialInstaller(mod: Mod): SpecialInstaller | null {
  const title = mod.title.trim().toLowerCase();
  if (title === "steamodded" || catalogInstallId(mod) === STEAMODDED_ID) {
    return "steamodded";
  }
  if (title === "talisman") return "talisman";
  return null;
}

/** Whether the detail view offers a Thunderstore / index switch for this mod */
export function hasSourceSwitch(mod: Mod): boolean {
  return (
    mod.source === "merged" &&
    Boolean(mod.thunderstore) &&
    specialInstaller(mod) === null
  );
}

export function indexDownloadUrl(mod: Mod): string {
  const url = mod.downloadURL?.trim();
  if (url) return url;
  return mod._dirName ? `index://${mod._dirName}` : "";
}

/**
 * What a reinstall of `mod` would do, null when it cannot be reinstalled
 * (not installed, local or manual installs, unknown installed version)
 */
export function reinstallTarget(
  mod: Mod,
  row?: InstalledMod | null,
  version?: string | null,
): ReinstallTarget | null {
  if (!row) return null;
  const installed = row.current_version?.trim() || null;

  const special = specialInstaller(mod);
  if (special && row.source !== "thunderstore") {
    const chosen = version?.trim();
    const tag = chosen && chosen !== "newest" ? chosen : installed;
    return tag ? { kind: special, row, version: tag } : null;
  }

  if (isInstalledPlaceholder(mod) || !catalogInstallId(mod)) return null;

  if (row.source === "thunderstore") {
    if (!usesCatalogInstall(mod) || !installed) return null;
    return { kind: "thunderstore", row, version: installed };
  }

  if (mod.source === "thunderstore") return null;
  const url = indexDownloadUrl(mod);
  if (!url) return null;
  return {
    kind: "index",
    row,
    version: mod.version?.trim() || null,
    url,
  };
}
