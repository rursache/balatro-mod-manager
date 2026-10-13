import { describe, it, expect } from "vitest";
import { CATALOG_SCHEMA, ensureCatalogSchema } from "./catalogSchema";

function memoryStorage(initial: Record<string, string> = {}): Storage {
  const data = new Map(Object.entries(initial));
  return {
    get length() {
      return data.size;
    },
    clear: () => data.clear(),
    getItem: (k) => data.get(k) ?? null,
    key: (i) => [...data.keys()][i] ?? null,
    removeItem: (k) => void data.delete(k),
    setItem: (k, v) => void data.set(k, String(v)),
  };
}

describe("ensureCatalogSchema", () => {
  it("clears the BMI era caches and sets the marker", () => {
    const storage = memoryStorage({
      "mods-cache": "[]",
      "mods-cache-ts": "1",
      "mods-descriptions-cache": "{}",
      currentCategory: "All Mods",
    });
    expect(ensureCatalogSchema(storage)).toBe(true);
    expect(storage.getItem("mods-cache")).toBeNull();
    expect(storage.getItem("mods-cache-ts")).toBeNull();
    expect(storage.getItem("mods-descriptions-cache")).toBeNull();
    expect(storage.getItem("currentCategory")).toBe("All Mods");
    expect(storage.getItem("catalog-schema")).toBe(CATALOG_SCHEMA);
  });

  it("drops the index only catalog cached before Thunderstore merging", () => {
    const storage = memoryStorage({
      "catalog-schema": "github:kasimeka/balatro-mod-index@main#1",
      "mods-cache": "[]",
      "mods-cache-ts": "1",
      "mods-descriptions-cache": "{}",
      "catalog.showNsfw": "true",
    });
    expect(CATALOG_SCHEMA).toContain("thunderstore");
    expect(ensureCatalogSchema(storage)).toBe(true);
    expect(storage.getItem("mods-cache")).toBeNull();
    expect(storage.getItem("mods-descriptions-cache")).toBeNull();
    expect(storage.getItem("catalog.showNsfw")).toBe("true");
    expect(storage.getItem("catalog-schema")).toBe(CATALOG_SCHEMA);
  });

  it("is idempotent", () => {
    const storage = memoryStorage();
    ensureCatalogSchema(storage);
    storage.setItem("mods-cache", "[1]");
    expect(ensureCatalogSchema(storage)).toBe(false);
    expect(storage.getItem("mods-cache")).toBe("[1]");
  });

  it("tolerates a throwing storage", () => {
    const storage = memoryStorage();
    storage.getItem = () => {
      throw new Error("blocked");
    };
    expect(ensureCatalogSchema(storage)).toBe(false);
  });
});
