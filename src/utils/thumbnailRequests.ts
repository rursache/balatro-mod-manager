import { invoke } from "@tauri-apps/api/core";

/** Matches the `.mod-card` height and `.mods-grid` gap at a card scale of 1 */
export const CARD_HEIGHT_PX = 330;
export const GRID_GAP_PX = 30;

/**
 * IntersectionObserver rootMargin that reaches one card row past the visible
 * area in both directions, so the next row starts loading before it scrolls in
 */
export function rowPreloadMargin(cardScale: number): string {
  const scale = Number.isFinite(cardScale) && cardScale > 0 ? cardScale : 1;
  const row = Math.round(CARD_HEIGHT_PX * scale + GRID_GAP_PX);
  return `${row}px 0px`;
}

export interface ThumbnailRequest {
  /** Resolves with the cached file path, or null when there is no thumbnail */
  promise: Promise<string | null>;
  /** Gives up on the request, the backend skips the download once nobody wants it */
  release: () => void;
}

interface PendingThumb {
  promise: Promise<string | null>;
  refs: number;
  cancelled: boolean;
}

/** How long a failed download is not retried when its card scrolls back in */
export const FAILED_RETRY_MS = 5 * 60 * 1000;

const pending = new Map<string, PendingThumb>();
const failedAt = new Map<string, number>();

/** Backend cache key, the catalog id when known so same named mods never share art */
export function thumbnailKey(title: string, id?: string | null): string {
  const trimmed = id?.trim();
  return trimmed ? trimmed : title;
}

export function requestThumbnail(
  title: string,
  url: string,
  id?: string | null,
): ThumbnailRequest {
  const key = thumbnailKey(title, id);
  const args = key === title ? { title } : { title, id: key };
  const failedTime = failedAt.get(key);
  if (failedTime !== undefined && Date.now() - failedTime < FAILED_RETRY_MS) {
    return { promise: Promise.resolve(null), release: () => {} };
  }
  let entry = pending.get(key);
  if (!entry) {
    const created: PendingThumb = {
      promise: Promise.resolve(null),
      refs: 0,
      cancelled: false,
    };
    created.promise = invoke<string | null>("fetch_thumbnail", { ...args, url })
      .catch(() => null)
      .then((path) => {
        if (pending.get(key) === created) pending.delete(key);
        if (path) failedAt.delete(key);
        else if (!created.cancelled) failedAt.set(key, Date.now());
        return path ?? null;
      });
    pending.set(key, created);
    entry = created;
  }
  const owned = entry;
  owned.refs += 1;
  let released = false;
  return {
    promise: owned.promise,
    release: () => {
      if (released) return;
      released = true;
      if (pending.get(key) !== owned) return;
      owned.refs -= 1;
      if (owned.refs > 0) return;
      owned.cancelled = true;
      pending.delete(key);
      invoke("cancel_thumbnail", args).catch(() => {});
    },
  };
}

/** Test helper, forgets pending and failed requests */
export function resetThumbnailRequests(): void {
  pending.clear();
  failedAt.clear();
}
