import { SortOption, type Mod } from "../stores/modStore";

const byName = (a: Mod, b: Mod) =>
  a.title.localeCompare(b.title, undefined, { sensitivity: "base" }) ||
  (a._dirName ?? a.title).localeCompare(b._dirName ?? b.title);

const downloadsOf = (m: Mod) =>
  typeof m.downloads === "number" && Number.isFinite(m.downloads)
    ? m.downloads
    : null;

/** Returns a sorted copy, mods without a known update time or download count always go last */
export function sortMods(mods: Mod[], sort: SortOption): Mod[] {
  const out = [...mods];
  switch (sort) {
    case SortOption.NameAsc:
      return out.sort(byName);
    case SortOption.NameDesc:
      return out.sort((a, b) => byName(b, a));
    case SortOption.LastUpdatedDesc:
    case SortOption.LastUpdatedAsc: {
      const dir = sort === SortOption.LastUpdatedDesc ? -1 : 1;
      return out.sort((a, b) => {
        const au = a.last_updated ?? 0;
        const bu = b.last_updated ?? 0;
        if (au === 0 || bu === 0) {
          if (au !== bu) return au === 0 ? 1 : -1;
          return byName(a, b);
        }
        return (au - bu) * dir || byName(a, b);
      });
    }
    case SortOption.DownloadsDesc:
      return out.sort((a, b) => {
        const ad = downloadsOf(a);
        const bd = downloadsOf(b);
        if (ad === null || bd === null) {
          if (ad !== bd) return ad === null ? 1 : -1;
          return byName(a, b);
        }
        return bd - ad || byName(a, b);
      });
    default:
      return out;
  }
}

const updatedAt = (m: Mod) =>
  typeof m.last_updated === "number" &&
  Number.isFinite(m.last_updated) &&
  m.last_updated > 0
    ? m.last_updated
    : null;

/** Reorders relevance-ranked search hits newest update first, keeping the relevance order for ties and undated mods */
export function sortSearchResults(ranked: Mod[]): Mod[] {
  // Array.prototype.sort is stable, so equal keys keep their relevance order
  return [...ranked].sort((a, b) => {
    const au = updatedAt(a);
    const bu = updatedAt(b);
    if (au === null || bu === null) {
      if (au === bu) return 0;
      return au === null ? 1 : -1;
    }
    return bu - au;
  });
}
