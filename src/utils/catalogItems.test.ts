import { describe, expect, it } from "vitest";
import {
  catalogSourceFields,
  usesCatalogInstall,
  type ArchiveModItem,
} from "./catalogItems";
import type { Mod, ThunderstoreInfo } from "../stores/modStore";

const tsInfo = (
  fullName: string,
  extra: Partial<ThunderstoreInfo> = {},
): ThunderstoreInfo => {
  const [namespace, name] = fullName.split("-");
  return {
    namespace,
    name,
    full_name: fullName,
    owner: namespace,
    package_url: `https://thunderstore.io/c/balatro/p/${namespace}/${name}/`,
    latest_version: "1.0.0",
    versions: [],
    download_url: "",
    website_url: "",
    icon_url: "",
    downloads_total: 42,
    rating: 0,
    deprecated: false,
    nsfw: false,
    ai_generated: false,
    categories_raw: [],
    date_updated: "",
    description: "",
    ...extra,
  };
};

const item = (
  dir: string,
  title: string,
  extra: Partial<ArchiveModItem> = {},
): ArchiveModItem => ({
  dir_name: dir,
  meta: { title, categories: [], author: "a", repo: "" },
  description: "",
  image_url: "",
  ...extra,
});

describe("catalogSourceFields", () => {
  it("maps merged entries and their flags", () => {
    const fields = catalogSourceFields(
      item("a@b", "B", {
        source: "merged",
        downloads: 1200,
        thunderstore: tsInfo("a-B", { ai_generated: true }),
      }),
    );
    expect(fields.source).toBe("merged");
    expect(fields.sources).toEqual(["index", "thunderstore"]);
    expect(fields.downloads).toBe(1200);
    expect(fields.aiGenerated).toBe(true);
    expect(fields.nsfw).toBe(false);
    expect(fields.deprecated).toBe(false);
  });

  it("defaults old index items without source fields", () => {
    const fields = catalogSourceFields(item("a@b", "B"));
    expect(fields.source).toBe("index");
    expect(fields.sources).toEqual(["index"]);
    expect(fields.downloads).toBeNull();
    expect(fields.thunderstore).toBeNull();
  });

  it("falls back to the package total for downloads", () => {
    const fields = catalogSourceFields(
      item("ts:a-B", "B", {
        source: "thunderstore",
        thunderstore: tsInfo("a-B", { deprecated: true, nsfw: true }),
      }),
    );
    expect(fields.downloads).toBe(42);
    expect(fields.deprecated).toBe(true);
    expect(fields.nsfw).toBe(true);
  });
});

describe("usesCatalogInstall", () => {
  const base: Mod = {
    title: "X",
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
  };

  it("routes Thunderstore backed cards through the planner", () => {
    expect(
      usesCatalogInstall({
        ...base,
        _dirName: "ts:a-X",
        source: "thunderstore",
        thunderstore: tsInfo("a-X"),
      }),
    ).toBe(true);
    expect(
      usesCatalogInstall({
        ...base,
        _dirName: "a@X",
        source: "merged",
        thunderstore: tsInfo("a-X"),
      }),
    ).toBe(true);
  });

  it("keeps index cards and Steamodded on their own installers", () => {
    expect(
      usesCatalogInstall({ ...base, _dirName: "a@X", source: "index" }),
    ).toBe(false);
    expect(
      usesCatalogInstall({
        ...base,
        _dirName: "Steamodded@smods",
        source: "merged",
        thunderstore: tsInfo("Steamodded-Steamodded"),
      }),
    ).toBe(false);
  });
});
