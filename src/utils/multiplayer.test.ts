import { describe, expect, it } from "vitest";
import {
  defaultTrack,
  gamemodesFor,
  isModeSelectable,
  legacyMigration,
  lovelyCheck,
  primaryAction,
  steamoddedMinimum,
  switchesTrack,
  type PackagePlan,
} from "./multiplayer";
import {
  manualLegacyCopy,
  modeFixture,
  statusFixture,
  trackFixture,
} from "../test/multiplayerFixtures";

const speedrun = "BalatroMultiplayer-MultiplayerSpeedrun";

describe("legacyMigration", () => {
  it("is offered for a manual GitHub install", () => {
    const status = statusFixture({
      legacy_manual: manualLegacyCopy(),
      tracks: [
        trackFixture({ track: "legacy", installed: true, official: false }),
        trackFixture({ track: "api" }),
      ],
    });
    expect(legacyMigration(status)).toEqual({
      installedVersion: "0.5.4",
      latestVersion: "0.5.7",
      available: true,
    });
  });

  it("is not offered without a manual copy", () => {
    expect(legacyMigration(statusFixture())).toBeNull();
  });

  it("is not offered for the official package", () => {
    const status = statusFixture({
      legacy_manual: manualLegacyCopy({ official: true }),
    });
    expect(legacyMigration(status)).toBeNull();
  });

  it("stays visible but unavailable while Thunderstore is offline", () => {
    const status = statusFixture({
      legacy_manual: manualLegacyCopy(),
      listing_available: false,
    });
    expect(legacyMigration(status)?.available).toBe(false);
  });

  it("falls back to the tracked version", () => {
    const status = statusFixture({
      legacy_manual: manualLegacyCopy({ mod_version: null }),
    });
    expect(legacyMigration(status)?.installedVersion).toBe("v0.5.4");
  });
});

describe("game mode selection", () => {
  it("only allows published modes on the API track", () => {
    const mode = modeFixture();
    expect(isModeSelectable(mode, "api", true)).toBe(true);
    expect(isModeSelectable(mode, "legacy", true)).toBe(false);
    expect(isModeSelectable(mode, "api", false)).toBe(false);
  });

  it("disables coming soon, deprecated and installed modes", () => {
    expect(
      isModeSelectable(
        modeFixture({ coming_soon: true, full_name: null }),
        "api",
        true,
      ),
    ).toBe(false);
    expect(
      isModeSelectable(modeFixture({ deprecated: true }), "api", true),
    ).toBe(false);
    expect(
      isModeSelectable(modeFixture({ installed: true }), "api", true),
    ).toBe(false);
    expect(
      isModeSelectable(modeFixture({ latest_version: null }), "api", true),
    ).toBe(false);
  });

  it("keeps installed modes and drops anything not selectable", () => {
    const status = statusFixture({
      gamemodes: [
        modeFixture({
          full_name: "BalatroMultiplayer-Installed",
          installed: true,
        }),
        modeFixture(),
        modeFixture({ full_name: null, coming_soon: true }),
      ],
    });
    expect(gamemodesFor(status, "api", [speedrun, "Unknown-Mode"])).toEqual([
      "BalatroMultiplayer-Installed",
      speedrun,
    ]);
    expect(gamemodesFor(status, "legacy", [speedrun])).toEqual([]);
  });
});

describe("track switching", () => {
  it("is a switch when the other track is installed", () => {
    const status = statusFixture({
      active_track: "legacy",
      tracks: [
        trackFixture({ track: "legacy", installed: true, official: true }),
        trackFixture({ track: "api" }),
      ],
    });
    expect(switchesTrack(status, "api")).toBe(true);
    expect(switchesTrack(status, "legacy")).toBe(false);
    expect(primaryAction(status, "api", []).label).toBe("Switch to this track");
  });

  it("treats installed game modes as part of the API track", () => {
    const status = statusFixture({
      gamemodes: [modeFixture({ installed: true })],
    });
    expect(switchesTrack(status, "legacy")).toBe(true);
  });
});

describe("primaryAction", () => {
  it("installs a fresh track", () => {
    expect(primaryAction(statusFixture(), "api", [])).toEqual({
      kind: "install",
      label: "Install",
      enabled: true,
    });
  });

  it("is disabled while the listing is unavailable", () => {
    const status = statusFixture({ listing_available: false });
    expect(primaryAction(status, "api", []).enabled).toBe(false);
  });

  it("offers updates for the track and its modes", () => {
    const tracks = [
      trackFixture({ track: "legacy" }),
      trackFixture({
        track: "api",
        installed: true,
        official: true,
        installed_version: "0.1.2",
      }),
    ];
    expect(
      primaryAction(statusFixture({ tracks }), "api", [speedrun]).kind,
    ).toBe("add_modes");
    expect(primaryAction(statusFixture({ tracks }), "api", []).kind).toBe(
      "none",
    );
    const outdated = statusFixture({
      tracks,
      gamemodes: [modeFixture({ installed: true, update_available: true })],
    });
    expect(primaryAction(outdated, "api", []).kind).toBe("update");
  });

  it("replaces a manual copy", () => {
    const status = statusFixture({
      tracks: [
        trackFixture({ track: "legacy", installed: true, official: false }),
        trackFixture({ track: "api" }),
      ],
    });
    expect(primaryAction(status, "legacy", []).kind).toBe("replace");
  });
});

describe("defaultTrack", () => {
  it("prefers the active track, then a manual legacy copy", () => {
    expect(defaultTrack(statusFixture({ active_track: "legacy" }))).toBe(
      "legacy",
    );
    expect(
      defaultTrack(statusFixture({ legacy_manual: manualLegacyCopy() })),
    ).toBe("legacy");
    expect(defaultTrack(statusFixture())).toBe("api");
  });
});

describe("lovelyCheck", () => {
  it("passes when the minimum is met", () => {
    expect(
      lovelyCheck(
        { min_version: "0.7.1", installed_version: "0.8.0", satisfied: true },
        true,
      ).status,
    ).toBe("ok");
  });

  it("explains an old or unknown version", () => {
    const old = lovelyCheck(
      { min_version: "0.7.1", installed_version: "0.6.0", satisfied: false },
      true,
    );
    expect(old.status).toBe("error");
    expect(old.detail).toContain("0.7.1");
    expect(
      lovelyCheck(
        { min_version: "0.7.1", installed_version: null, satisfied: false },
        true,
      ).detail,
    ).toContain("unknown");
  });

  it("falls back to presence without a requirement", () => {
    expect(lovelyCheck(null, true).status).toBe("ok");
    expect(lovelyCheck(null, false).status).toBe("error");
    expect(lovelyCheck(null, null).status).toBe("warning");
  });
});

describe("steamoddedMinimum", () => {
  const plan = (steamodded: PackagePlan["steamodded"]): PackagePlan => ({
    mod_id: "ts:BalatroMultiplayer-MultiplayerAPI",
    mod_title: "MultiplayerAPI",
    version: "0.1.2",
    lovely: null,
    steamodded,
    missing: [],
    warnings: [],
  });

  it("reports the release tag of an unmet minimum", () => {
    expect(
      steamoddedMinimum([
        plan({
          min_version: "1.1814.0",
          min_tag: "1.0.0-beta-1814a",
          installed: false,
          installed_version: null,
          satisfied: false,
          legacy_api: false,
        }),
      ]),
    ).toBe("1.0.0-beta-1814a");
  });

  it("is null when Steamodded is fine", () => {
    expect(
      steamoddedMinimum([
        plan({
          min_version: "1.1814.0",
          min_tag: "1.0.0-beta-1814a",
          installed: true,
          installed_version: "1.0.0-beta-1815a",
          satisfied: true,
          legacy_api: false,
        }),
        plan(null),
      ]),
    ).toBeNull();
  });
});
