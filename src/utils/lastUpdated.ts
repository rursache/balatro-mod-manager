/** Date of a mod's `last_updated` value, or null when the index has none (missing or 0) */
export function lastUpdatedDate(value?: number | null): Date | null {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    return null;
  }
  // The index stores unix seconds, anything this large is already in ms
  const ms = value < 1e12 ? value * 1000 : value;
  const date = new Date(ms);
  return Number.isNaN(date.getTime()) ? null : date;
}

export function formatLastUpdated(
  value?: number | null,
  locale?: string,
): string | null {
  const date = lastUpdatedDate(value);
  if (!date) return null;
  return date.toLocaleDateString(locale, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}
