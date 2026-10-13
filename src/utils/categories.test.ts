import { describe, expect, it } from "vitest";
import { categoryFromName, categoryLabel, mapCategories } from "./categories";
import { Category } from "../stores/modStore";

describe("mapCategories", () => {
  it("maps index categories case insensitively", () => {
    expect(
      mapCategories(["Content", "joker", "Quality of Life", "Resources"]),
    ).toEqual([
      Category.Content,
      Category.Joker,
      Category.QualityOfLife,
      Category.ResourcePacks,
    ]);
    expect(mapCategories(["Extension"])).toEqual([Category.API]);
  });

  it("maps Thunderstore community categories", () => {
    expect(mapCategories(["Content Mods", "New Mechanics"])).toEqual([
      Category.Content,
    ]);
    expect(mapCategories(["Tools"])).toEqual([Category.Technical]);
    expect(mapCategories(["Libraries"])).toEqual([Category.API]);
    expect(mapCategories(["Textures", "Audio"])).toEqual([
      Category.ResourcePacks,
    ]);
    expect(
      mapCategories(["Gameplay Tweaks", "Misc", "Translation", "Text Changes"]),
    ).toEqual([Category.Miscellaneous]);
  });

  it("only counts the generic Mods tag when nothing else mapped", () => {
    expect(mapCategories(["Mods"])).toEqual([Category.Miscellaneous]);
    expect(mapCategories(["Mods", "Tools"])).toEqual([Category.Technical]);
  });

  it("never turns AI Generated or unknown names into a category", () => {
    expect(mapCategories(["AI Generated"])).toEqual([]);
    expect(mapCategories(["Something Else", ""])).toEqual([]);
    expect(mapCategories(null)).toEqual([]);
  });
});

describe("category names", () => {
  it("round trips labels", () => {
    for (const cat of [
      Category.Content,
      Category.Joker,
      Category.QualityOfLife,
      Category.Technical,
      Category.Miscellaneous,
      Category.ResourcePacks,
      Category.API,
    ]) {
      expect(categoryFromName(categoryLabel(cat))).toBe(cat);
    }
  });
});
