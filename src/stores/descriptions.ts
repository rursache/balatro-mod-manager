import { writable } from "svelte/store";
import { ensureCatalogSchema } from "../utils/catalogSchema";

export const descriptionsStore = writable<Record<string, string>>({});

export function setDescription(title: string, description: string) {
  descriptionsStore.update((map) => {
    map[title] = description;
    return map;
  });
}

export function setDescriptions(batch: Record<string, string>) {
  if (!batch || Object.keys(batch).length === 0) return;
  descriptionsStore.update((map) => {
    Object.assign(map, batch);
    return map;
  });
}

const CACHE_KEY = "mods-descriptions-cache";
let latestValue: Record<string, string> = {};
let persistTimer: ReturnType<typeof setTimeout> | null = null;
let persistScheduled = false;
let persistSuspended = false;
let persistPending = false;

function schedulePersist() {
  if (typeof window === "undefined") return;
  if (persistSuspended) {
    persistPending = true;
    return;
  }
  if (persistScheduled) return;
  persistScheduled = true;
  const runPersist = () => {
    persistScheduled = false;
    try {
      window.localStorage.setItem(CACHE_KEY, JSON.stringify(latestValue));
    } catch {
      // ignore quota errors
    }
  };
  if ("requestIdleCallback" in window) {
    (
      window as Window & {
        requestIdleCallback: (
          _cb: () => void,
          _opts?: { timeout: number },
        ) => number;
      }
    ).requestIdleCallback(runPersist, { timeout: 2000 });
  } else {
    persistTimer = setTimeout(runPersist, 1500);
  }
}

export async function withDescriptionsPersistenceSuspended<T>(
  fn: () => Promise<T>,
) {
  persistSuspended = true;
  try {
    return await fn();
  } finally {
    persistSuspended = false;
    if (persistPending) {
      persistPending = false;
      schedulePersist();
    }
  }
}

// Hydrate from localStorage (best-effort)
if (typeof window !== "undefined") {
  try {
    ensureCatalogSchema();
    const raw = window.localStorage.getItem(CACHE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed === "object") {
        descriptionsStore.set(parsed);
      }
    }
  } catch {
    // ignore malformed cache
  }

  // Persist with a debounce to avoid thrashing during hydration
  descriptionsStore.subscribe((val) => {
    latestValue = val;
    if (persistTimer) clearTimeout(persistTimer);
    schedulePersist();
  });
}
