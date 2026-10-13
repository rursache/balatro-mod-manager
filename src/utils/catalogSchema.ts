export const CATALOG_SCHEMA = "github:kasimeka/balatro-mod-index@main#1";

const SCHEMA_KEY = "catalog-schema";
const STALE_KEYS = ["mods-cache", "mods-cache-ts", "mods-descriptions-cache"];

/**
 * Drops catalog caches written for another catalog source (the old BMI server)
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
