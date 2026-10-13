export const CATALOG_SCHEMA =
  "github:kasimeka/balatro-mod-index@main+thunderstore:balatro#2";

const SCHEMA_KEY = "catalog-schema";
const STALE_KEYS = ["mods-cache", "mods-cache-ts", "mods-descriptions-cache"];

/**
 * Drops catalog caches written for another catalog shape (the old BMI server, or
 * the index only catalog before Thunderstore was merged in)
 * Returns true when caches were cleared
 */
export function ensureCatalogSchema(storage?: Storage): boolean {
  try {
    storage ??= localStorage;
    if (storage.getItem(SCHEMA_KEY) === CATALOG_SCHEMA) return false;
    for (const key of STALE_KEYS) storage.removeItem(key);
    storage.setItem(SCHEMA_KEY, CATALOG_SCHEMA);
    return true;
  } catch {
    return false;
  }
}
