import type {
  GameModeInfo,
  InstalledCopy,
  MultiplayerStatus,
  TrackInfo,
} from "../utils/multiplayer";

export function trackFixture(overrides: Partial<TrackInfo> = {}): TrackInfo {
  const track = overrides.track ?? "legacy";
  return {
    track,
    full_name:
      track === "legacy"
        ? "BalatroMultiplayer-Multiplayer"
        : "BalatroMultiplayer-MultiplayerAPI",
    title: track === "legacy" ? "Multiplayer" : "Multiplayer API",
    description: "",
    latest_version: track === "legacy" ? "0.5.7" : "0.1.2",
    deprecated: false,
    icon_url: "",
    package_url: `https://thunderstore.io/c/balatro/p/BalatroMultiplayer/${track}/`,
    installed: false,
    installed_version: null,
    tracked_name: null,
    official: false,
    disabled: false,
    path: null,
    update_available: false,
    ...overrides,
  };
}

export function modeFixture(
  overrides: Partial<GameModeInfo> = {},
): GameModeInfo {
  return {
    full_name: "BalatroMultiplayer-MultiplayerSpeedrun",
    mod_id: "MultiplayerSPDRN",
    title: "Speedrun",
    description: "Race your friends",
    latest_version: "0.1.1",
    deprecated: false,
    icon_url: "",
    package_url: "",
    installed: false,
    installed_version: null,
    tracked_name: null,
    update_available: false,
    coming_soon: false,
    ...overrides,
  };
}

export function manualLegacyCopy(
  overrides: Partial<InstalledCopy> = {},
): InstalledCopy {
  return {
    role: "legacy",
    path: "/mods/Multiplayer",
    folder: "Multiplayer",
    mod_id: "Multiplayer",
    mod_version: "0.5.4",
    package: null,
    package_version: null,
    tracked_name: "Multiplayer",
    tracked_source: "index",
    tracked_version: "v0.5.4",
    official: false,
    disabled: false,
    ...overrides,
  };
}

export function statusFixture(
  overrides: Partial<MultiplayerStatus> = {},
): MultiplayerStatus {
  return {
    namespace: "BalatroMultiplayer",
    casual_only: true,
    bet_download_url: "https://new.balatromp.com/download",
    listing_available: true,
    listing_error: null,
    platform: "macos",
    tracks: [trackFixture({ track: "legacy" }), trackFixture({ track: "api" })],
    active_track: null,
    conflict: false,
    legacy_manual: null,
    gamemodes: [
      modeFixture(),
      modeFixture({
        full_name: null,
        mod_id: "PvP",
        title: "PvP",
        description: "",
        latest_version: null,
        coming_soon: true,
      }),
    ],
    readiness: [],
    copied_dlls: [],
    ...overrides,
  };
}
