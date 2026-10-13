// Shapes returned by the multiplayer_* commands (src-tauri/src/commands/multiplayer.rs)

export type Track = "legacy" | "api";
export type CopyRole = "legacy" | "api" | "gamemode";
export type CheckStatus = "ok" | "warning" | "error";
export type LaunchPlatform = "macos" | "windows" | "linux" | "linux_wine";

export interface TrackInfo {
  track: Track;
  full_name: string;
  title: string;
  description: string;
  latest_version: string | null;
  deprecated: boolean;
  icon_url: string;
  package_url: string;
  installed: boolean;
  installed_version: string | null;
  tracked_name: string | null;
  official: boolean;
  disabled: boolean;
  path: string | null;
  update_available: boolean;
}

export interface GameModeInfo {
  full_name: string | null;
  mod_id: string | null;
  title: string;
  description: string;
  latest_version: string | null;
  deprecated: boolean;
  icon_url: string;
  package_url: string;
  installed: boolean;
  installed_version: string | null;
  tracked_name: string | null;
  update_available: boolean;
  coming_soon: boolean;
}

export interface ReadinessCheck {
  id: string;
  label: string;
  status: CheckStatus;
  detail: string | null;
}

export interface InstalledCopy {
  role: CopyRole;
  path: string;
  folder: string;
  mod_id: string;
  mod_version: string | null;
  package: string | null;
  package_version: string | null;
  tracked_name: string | null;
  tracked_source: string | null;
  tracked_version: string | null;
  official: boolean;
  disabled: boolean;
}

export interface MultiplayerStatus {
  namespace: string;
  casual_only: boolean;
  bet_download_url: string;
  listing_available: boolean;
  listing_error: string | null;
  platform: LaunchPlatform;
  tracks: TrackInfo[];
  active_track: Track | null;
  conflict: boolean;
  legacy_manual: InstalledCopy | null;
  gamemodes: GameModeInfo[];
  /** What uninstalling each track removes and leaves alone */
  uninstall: UninstallPreview[];
  readiness: ReadinessCheck[];
  copied_dlls: string[];
}

export interface Removal {
  role: CopyRole;
  path: string;
  folder: string;
  tracked_name: string | null;
  replaced_by: string | null;
  reason: string;
}

export interface UninstallPreview {
  track: Track;
  remove: Removal[];
  /** Git checkouts and third party mods built on the API, never deleted */
  keep: Removal[];
}

export interface TrackPlan {
  track: Track;
  packages: string[];
  remove: Removal[];
  replace: Removal[];
  needs_confirmation: boolean;
  warnings: string[];
}

export interface LovelyRequirement {
  min_version: string;
  installed_version: string | null;
  satisfied: boolean;
}

export interface SteamoddedRequirement {
  min_version: string | null;
  min_tag: string | null;
  installed: boolean;
  installed_version: string | null;
  satisfied: boolean;
  legacy_api: boolean;
}

// The parts of install_plan::InstallPlan this view reads
export interface PackagePlan {
  mod_id: string;
  mod_title: string;
  version: string;
  lovely: LovelyRequirement | null;
  steamodded: SteamoddedRequirement | null;
  missing: string[];
  warnings: string[];
}

export interface InstalledCatalogMod {
  name: string;
  path: string;
  version: string;
  source: "index" | "thunderstore";
  source_id: string | null;
}

export interface MultiplayerInstallResult {
  track: Track;
  installed: InstalledCatalogMod[];
  removed: string[];
  plans: PackagePlan[];
  warnings: string[];
}

export interface MultiplayerUninstallResult {
  track: Track;
  removed: string[];
  removed_dlls: string[];
}

export const TRACKS: Track[] = ["legacy", "api"];

export function otherTrack(track: Track): Track {
  return track === "legacy" ? "api" : "legacy";
}

export function trackInfo(
  status: MultiplayerStatus,
  track: Track,
): TrackInfo | null {
  return status.tracks.find((t) => t.track === track) ?? null;
}

export function defaultTrack(status: MultiplayerStatus): Track {
  if (status.active_track) return status.active_track;
  if (status.legacy_manual) return "legacy";
  return "api";
}

// A mode the user can tick for the next install
export function isModeSelectable(
  mode: GameModeInfo,
  track: Track,
  listingAvailable: boolean,
): boolean {
  if (track !== "api" || !listingAvailable) return false;
  if (mode.coming_soon || !mode.full_name || !mode.latest_version) return false;
  if (mode.installed) return false;
  return !mode.deprecated;
}

// Installed modes are always passed along so an install or update keeps them current
export function gamemodesFor(
  status: MultiplayerStatus,
  track: Track,
  selected: string[],
): string[] {
  if (track !== "api") return [];
  const out: string[] = [];
  const add = (name: string | null) => {
    if (!name) return;
    if (!out.some((n) => n.toLowerCase() === name.toLowerCase()))
      out.push(name);
  };
  for (const mode of status.gamemodes) {
    if (mode.installed && mode.full_name) add(mode.full_name);
  }
  for (const name of selected) {
    const mode = status.gamemodes.find(
      (m) => m.full_name?.toLowerCase() === name.toLowerCase(),
    );
    if (mode && isModeSelectable(mode, track, status.listing_available))
      add(mode.full_name);
  }
  return out;
}

// True when installing `track` would remove what is installed for the other one
export function switchesTrack(
  status: MultiplayerStatus,
  track: Track,
): boolean {
  if (trackInfo(status, otherTrack(track))?.installed) return true;
  return track === "legacy" && status.gamemodes.some((m) => m.installed);
}

export type PrimaryKind =
  "install" | "replace" | "update" | "add_modes" | "none";

export interface PrimaryAction {
  kind: PrimaryKind;
  label: string;
  enabled: boolean;
}

export function primaryAction(
  status: MultiplayerStatus,
  track: Track,
  selected: string[],
): PrimaryAction {
  const info = trackInfo(status, track);
  const available =
    status.listing_available && !!info?.latest_version && !info.deprecated;
  if (!info || !info.installed) {
    return {
      kind: "install",
      label: switchesTrack(status, track) ? "Switch to this track" : "Install",
      enabled: available,
    };
  }
  if (!info.official) {
    return { kind: "replace", label: "Install official", enabled: available };
  }
  const modesOutdated =
    track === "api" && status.gamemodes.some((m) => m.update_available);
  if (info.update_available || modesOutdated) {
    return { kind: "update", label: "Update", enabled: available };
  }
  const newModes = gamemodesFor(status, track, selected).filter(
    (name) =>
      !status.gamemodes.some(
        (m) => m.installed && m.full_name?.toLowerCase() === name.toLowerCase(),
      ),
  );
  if (newModes.length > 0) {
    return {
      kind: "add_modes",
      label: newModes.length === 1 ? "Add game mode" : "Add game modes",
      enabled: available,
    };
  }
  return { kind: "none", label: "Up to date", enabled: false };
}

export interface LegacyMigration {
  installedVersion: string | null;
  latestVersion: string | null;
  available: boolean;
}

// The old GitHub release (Mods/Multiplayer) is offered an upgrade to the official package
export function legacyMigration(
  status: MultiplayerStatus,
): LegacyMigration | null {
  const manual = status.legacy_manual;
  if (!manual || manual.official || manual.role !== "legacy") return null;
  const legacy = trackInfo(status, "legacy");
  const latestVersion = legacy?.latest_version ?? null;
  return {
    installedVersion: manual.mod_version ?? manual.tracked_version ?? null,
    latestVersion,
    available:
      status.listing_available && !!latestVersion && !legacy?.deprecated,
  };
}

export function lovelyCheck(
  requirement: LovelyRequirement | null,
  installed: boolean | null,
): ReadinessCheck {
  const base = { id: "lovely", label: "Lovely" };
  if (requirement?.satisfied) {
    return {
      ...base,
      status: "ok",
      detail: requirement.installed_version,
    };
  }
  if (requirement) {
    if (installed === false) {
      return {
        ...base,
        status: "error",
        detail: `Lovely is not installed, needs ${requirement.min_version} or newer`,
      };
    }
    return {
      ...base,
      status: "error",
      detail: requirement.installed_version
        ? `Needs Lovely ${requirement.min_version} or newer, ${requirement.installed_version} is installed`
        : `Needs Lovely ${requirement.min_version} or newer, the installed version is unknown`,
    };
  }
  if (installed === null) {
    return { ...base, status: "warning", detail: "Could not check Lovely" };
  }
  return installed
    ? { ...base, status: "ok", detail: null }
    : { ...base, status: "error", detail: "Lovely is not installed" };
}

export function steamoddedMinimum(plans: PackagePlan[]): string | null {
  for (const plan of plans) {
    const s = plan.steamodded;
    if (s && !s.satisfied) return s.min_tag ?? s.min_version;
  }
  return null;
}

export function removalLabel(removal: Removal): string {
  return removal.tracked_name ?? removal.folder;
}
