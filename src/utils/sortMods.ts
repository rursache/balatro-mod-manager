import { SortOption, type Mod } from "../stores/modStore";

const byName = (a: Mod, b: Mod) =>
  a.title.localeCompare(b.title, undefined, { sensitivity: "base" }) ||
  (a._dirName ?? a.title).localeCompare(b._dirName ?? b.title);

/** Returns a sorted copy, mods without a known update time always go last */
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
    default:
      return out;
  }
}
