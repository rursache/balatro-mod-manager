import { Category } from "../stores/modStore";

/** Index and Thunderstore category names, matched case insensitively */
const CATEGORY_NAMES: Record<string, Category> = {
  content: Category.Content,
  joker: Category.Joker,
  "quality of life": Category.QualityOfLife,
  technical: Category.Technical,
  miscellaneous: Category.Miscellaneous,
  "resource packs": Category.ResourcePacks,
  resources: Category.ResourcePacks,
  api: Category.API,
  // Newer index schema category without its own tab yet
  extension: Category.API,
  // Thunderstore community categories
  "content mods": Category.Content,
  "new mechanics": Category.Content,
  tools: Category.Technical,
  libraries: Category.API,
  textures: Category.ResourcePacks,
  audio: Category.ResourcePacks,
  "gameplay tweaks": Category.Miscellaneous,
  misc: Category.Miscellaneous,
  translation: Category.Miscellaneous,
  "text changes": Category.Miscellaneous,
  mods: Category.Miscellaneous,
};

/** Tab label of each category, also used by the sidebar */
export const CATEGORY_LABELS: Record<Category, string> = {
  [Category.Content]: "Content",
  [Category.Joker]: "Joker",
  [Category.QualityOfLife]: "Quality of Life",
  [Category.Technical]: "Technical",
  [Category.Miscellaneous]: "Miscellaneous",
  [Category.ResourcePacks]: "Resource Packs",
  [Category.API]: "API",
};

export function categoryFromName(name: string): Category | null {
  return CATEGORY_NAMES[name.trim().toLowerCase()] ?? null;
}

export function categoryLabel(category: Category): string {
  return CATEGORY_LABELS[category] ?? "All Mods";
}

/**
 * Maps raw category names to app categories, keeping the first occurrence order.
 * The catch all Thunderstore `Mods` tag only counts when nothing else mapped and
 * "AI Generated" is a badge, never a category
 */
export function mapCategories(names?: readonly string[] | null) {
  const out: Category[] = [];
  let sawGenericMods = false;
  for (const raw of names ?? []) {
    if (typeof raw !== "string") continue;
    const key = raw.trim().toLowerCase();
    if (key === "mods") {
      sawGenericMods = true;
      continue;
    }
    const cat = categoryFromName(key);
    if (cat !== null && !out.includes(cat)) out.push(cat);
  }
  if (out.length === 0 && sawGenericMods) out.push(Category.Miscellaneous);
  return out;
}
