import { describe, expect, it } from "vitest";
import {
  DEFAULT_CATALOG_FILTERS,
  formatDownloads,
  formatSyncStatus,
  passesCatalogFilters,
  syncHasProblem,
  type CatalogFilterOptions,
} from "./catalogFilters";
import type { Mod } from "../stores/modStore";
import type { CatalogStatus } from "../stores/catalogFilters";

const mod = (title: string, extra: Partial<Mod> = {}): Mod => ({
  title,
  description: "",
  image: "",
  categories: [],
  colors: { color1: "", color2: "" },
  requires_steamodded: false,
  requires_talisman: false,
  publisher: "",
  repo: "",
  downloadURL: "",
  installed: false,
  last_updated: 0,
  ...extra,
});

const all = [
  mod("Index Only", { source: "index" }),
  mod("Cached Before Sources"),
  mod("Merged", { source: "merged" }),
  mod("Package", { source: "thunderstore" }),
  mod("Spicy", { source: "thunderstore", nsfw: true }),
  mod("Slop", { source: "thunderstore", aiGenerated: true }),
  mod("Old", { source: "thunderstore", deprecated: true }),
];

const visible = (options: Partial<CatalogFilterOptions> = {}) =>
  all
    .filter((m) =>
      passesCatalogFilters(m, { ...DEFAULT_CATALOG_FILTERS, ...options }),
    )
    .map((m) => m.title);

describe("passesCatalogFilters", () => {
  it("hides NSFW and deprecated by default and shows AI tagged mods", () => {
    expect(visible()).toEqual([
      "Index Only",
      "Cached Before Sources",
      "Merged",
      "Package",
      "Slop",
    ]);
  });

  it("applies each toggle", () => {
    expect(visible({ showNsfw: true })).toContain("Spicy");
    expect(visible({ showDeprecated: true })).toContain("Old");
    expect(visible({ hideAiGenerated: true })).not.toContain("Slop");
  });

  it("filters by source chip", () => {
    expect(visible({ source: "index" })).toEqual([
      "Index Only",
      "Cached Before Sources",
      "Merged",
    ]);
    expect(visible({ source: "thunderstore" })).toEqual([
      "Merged",
      "Package",
      "Slop",
    ]);
  });

  it("drops Thunderstore only packages when the source is off", () => {
    expect(visible({ thunderstoreEnabled: false })).toEqual([
      "Index Only",
      "Cached Before Sources",
      "Merged",
    ]);
    expect(
      visible({ thunderstoreEnabled: false, source: "thunderstore" }),
    ).toEqual([]);
  });
});

describe("formatSyncStatus", () => {
  const now = 1_800_000_000_000;
  const nowSec = now / 1000;
  const status = (
    index: Partial<CatalogStatus["index"]>,
    thunderstore: Partial<CatalogStatus["thunderstore"]>,
  ): CatalogStatus => ({
    index: {
      ok: true,
      synced_at: nowSec - 300,
      error: null,
      count: 1,
      ...index,
    },
    thunderstore: {
      ok: true,
      synced_at: nowSec - 30,
      error: null,
      count: 1,
      ...thunderstore,
    },
  });

  it("describes healthy sources", () => {
    const s = status({}, {});
    expect(formatSyncStatus(s, now)).toBe(
      "Index synced 5m ago, Thunderstore synced just now",
    );
    expect(syncHasProblem(s)).toBe(false);
  });

  it("reports an offline source with its cache age", () => {
    const s = status({}, { ok: false, synced_at: nowSec - 7200, error: "x" });
    expect(formatSyncStatus(s, now)).toBe(
      "Index synced 5m ago, Thunderstore offline, cached 2h ago",
    );
    expect(syncHasProblem(s)).toBe(true);
    expect(syncHasProblem(s, false)).toBe(false);
    expect(formatSyncStatus(s, now, false)).toBe("Index synced 5m ago");
  });

  it("handles sources that never synced", () => {
    const s = status({ ok: false, synced_at: 0 }, { synced_at: 0 });
    expect(formatSyncStatus(s, now)).toBe(
      "Index unavailable, Thunderstore synced",
    );
    expect(formatSyncStatus(null, now)).toBe("");
  });
});

describe("formatDownloads", () => {
  it("formats compact counts and rejects unknown values", () => {
    expect(formatDownloads(950)).toBe("950");
    expect(formatDownloads(12_345)).toBe("12.3K");
    expect(formatDownloads(2_500_000)).toBe("2.5M");
    expect(formatDownloads(0)).toBe("0");
    expect(formatDownloads(null)).toBeNull();
    expect(formatDownloads()).toBeNull();
    expect(formatDownloads(Number.NaN)).toBeNull();
  });
});
