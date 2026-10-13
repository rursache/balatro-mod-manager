import { describe, it, expect } from "vitest";
import { sortMods, sortSearchResults } from "./sortMods";
import { SortOption, type Mod } from "../stores/modStore";

const mod = (title: string, last_updated = 0, dir?: string): Mod => ({
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
  last_updated,
  _dirName: dir,
});

const titles = (mods: Mod[]) => mods.map((m) => m.title);

describe("sortMods", () => {
  it("sorts by name case-insensitively with a dir name tiebreak", () => {
    const input = [
      mod("beta"),
      mod("Alpha", 0, "b@alpha"),
      mod("alpha", 0, "a@alpha"),
    ];
    const asc = sortMods(input, SortOption.NameAsc);
    expect(asc.map((m) => m._dirName ?? m.title)).toEqual([
      "a@alpha",
      "b@alpha",
      "beta",
    ]);
    expect(titles(sortMods(input, SortOption.NameDesc))).toEqual([
      "beta",
      "Alpha",
      "alpha",
    ]);
  });

  it("sorts by last updated and keeps unknown times last", () => {
    const input = [mod("old", 10), mod("none"), mod("new", 30), mod("mid", 20)];
    expect(titles(sortMods(input, SortOption.LastUpdatedDesc))).toEqual([
      "new",
      "mid",
      "old",
      "none",
    ]);
    expect(titles(sortMods(input, SortOption.LastUpdatedAsc))).toEqual([
      "old",
      "mid",
      "new",
      "none",
    ]);
  });

  it("breaks update time ties by name", () => {
    const input = [mod("b", 5), mod("a", 5), mod("d"), mod("c")];
    expect(titles(sortMods(input, SortOption.LastUpdatedDesc))).toEqual([
      "a",
      "b",
      "c",
      "d",
    ]);
  });

  it("sorts by downloads with unknown counts last", () => {
    const withDownloads = (title: string, downloads?: number | null) => ({
      ...mod(title),
      downloads,
    });
    const input = [
      withDownloads("index only"),
      withDownloads("small", 10),
      withDownloads("big", 5000),
      withDownloads("none", null),
      withDownloads("zero", 0),
      withDownloads("also big", 5000),
    ];
    expect(titles(sortMods(input, SortOption.DownloadsDesc))).toEqual([
      "also big",
      "big",
      "small",
      "zero",
      "index only",
      "none",
    ]);
  });

  it("does not mutate its input", () => {
    const input = [mod("b"), mod("a")];
    sortMods(input, SortOption.NameAsc);
    expect(titles(input)).toEqual(["b", "a"]);
  });
});

describe("sortSearchResults", () => {
  it("orders hits by most recent update first", () => {
    const ranked = [mod("best", 10), mod("good", 30), mod("ok", 20)];
    expect(titles(sortSearchResults(ranked))).toEqual(["good", "ok", "best"]);
  });

  it("keeps relevance order for equal update times", () => {
    const ranked = [mod("z first", 5), mod("a second", 5), mod("newer", 9)];
    expect(titles(sortSearchResults(ranked))).toEqual([
      "newer",
      "z first",
      "a second",
    ]);
  });

  it("puts undated mods last in relevance order", () => {
    const ranked = [
      mod("undated best"),
      mod("dated", 1),
      { ...mod("nan"), last_updated: Number.NaN },
      mod("undated worse"),
      mod("dated newer", 2),
    ];
    expect(titles(sortSearchResults(ranked))).toEqual([
      "dated newer",
      "dated",
      "undated best",
      "nan",
      "undated worse",
    ]);
  });

  it("does not mutate its input", () => {
    const ranked = [mod("old", 1), mod("new", 2)];
    sortSearchResults(ranked);
    expect(titles(ranked)).toEqual(["old", "new"]);
  });
});
