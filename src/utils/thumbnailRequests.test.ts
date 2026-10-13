import { describe, it, expect, vi, beforeEach } from "vitest";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

import {
  FAILED_RETRY_MS,
  requestThumbnail,
  resetThumbnailRequests,
  rowPreloadMargin,
} from "./thumbnailRequests";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

function callsTo(cmd: string) {
  return mockInvoke.mock.calls.filter(([name]) => name === cmd);
}

const URL =
  "https://raw.githubusercontent.com/x/y/abc/mods/A%40B/thumbnail.jpg";

describe("rowPreloadMargin", () => {
  it("covers one card row plus the grid gap", () => {
    expect(rowPreloadMargin(1)).toBe("360px 0px");
    expect(rowPreloadMargin(0.8)).toBe("294px 0px");
    expect(rowPreloadMargin(1.25)).toBe("443px 0px");
  });

  it("falls back to scale 1 for bad input", () => {
    expect(rowPreloadMargin(0)).toBe("360px 0px");
    expect(rowPreloadMargin(Number.NaN)).toBe("360px 0px");
  });
});

describe("requestThumbnail", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    mockInvoke.mockResolvedValue(undefined);
    resetThumbnailRequests();
  });

  it("fetches once per title while a request is pending", async () => {
    const fetch = deferred<string | null>();
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === "fetch_thumbnail" ? fetch.promise : Promise.resolve(),
    );

    const a = requestThumbnail("A", URL);
    const b = requestThumbnail("A", URL);
    expect(callsTo("fetch_thumbnail")).toEqual([
      ["fetch_thumbnail", { title: "A", url: URL }],
    ]);

    fetch.resolve("/cache/a.jpg");
    await expect(a.promise).resolves.toBe("/cache/a.jpg");
    await expect(b.promise).resolves.toBe("/cache/a.jpg");
  });

  it("cancels only after every card scrolled away", () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === "fetch_thumbnail" ? new Promise(() => {}) : Promise.resolve(),
    );

    const a = requestThumbnail("A", URL);
    const b = requestThumbnail("A", URL);
    a.release();
    a.release();
    expect(callsTo("cancel_thumbnail")).toHaveLength(0);
    b.release();
    expect(callsTo("cancel_thumbnail")).toEqual([
      ["cancel_thumbnail", { title: "A" }],
    ]);
  });

  it("starts a fresh fetch when a cancelled card comes back", () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === "fetch_thumbnail" ? new Promise(() => {}) : Promise.resolve(),
    );

    requestThumbnail("A", URL).release();
    requestThumbnail("A", URL);
    expect(callsTo("fetch_thumbnail")).toHaveLength(2);
    expect(callsTo("cancel_thumbnail")).toHaveLength(1);
  });

  it("does not cancel after the thumbnail already arrived", async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === "fetch_thumbnail"
        ? Promise.resolve("/cache/a.jpg")
        : Promise.resolve(),
    );

    const a = requestThumbnail("A", URL);
    await a.promise;
    a.release();
    expect(callsTo("cancel_thumbnail")).toHaveLength(0);
  });

  it("a stale release does not cancel a newer request", async () => {
    const first = deferred<string | null>();
    let calls = 0;
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd !== "fetch_thumbnail") return Promise.resolve();
      calls += 1;
      return calls === 1 ? first.promise : new Promise(() => {});
    });

    const old = requestThumbnail("A", URL);
    first.resolve("/cache/a.jpg");
    await old.promise;
    requestThumbnail("A", URL);
    old.release();
    expect(callsTo("cancel_thumbnail")).toHaveLength(0);
  });

  it("does not hit the network again for a thumbnail that failed", async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === "fetch_thumbnail" ? Promise.resolve(null) : Promise.resolve(),
    );

    const now = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
    try {
      await expect(requestThumbnail("A", URL).promise).resolves.toBeNull();
      await expect(requestThumbnail("A", URL).promise).resolves.toBeNull();
      expect(callsTo("fetch_thumbnail")).toHaveLength(1);

      now.mockReturnValue(1_000_000 + FAILED_RETRY_MS);
      await requestThumbnail("A", URL).promise;
      expect(callsTo("fetch_thumbnail")).toHaveLength(2);
    } finally {
      now.mockRestore();
    }
  });

  it("retries a thumbnail whose fetch was cancelled rather than failed", async () => {
    const first = deferred<string | null>();
    let calls = 0;
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd !== "fetch_thumbnail") return Promise.resolve();
      calls += 1;
      return calls === 1 ? first.promise : Promise.resolve("/cache/a.jpg");
    });

    const old = requestThumbnail("A", URL);
    old.release();
    first.resolve(null);
    await old.promise;

    await expect(requestThumbnail("A", URL).promise).resolves.toBe(
      "/cache/a.jpg",
    );
    expect(callsTo("fetch_thumbnail")).toHaveLength(2);
  });

  it("treats invoke errors as a missing thumbnail", async () => {
    mockInvoke.mockImplementation((cmd: string) =>
      cmd === "fetch_thumbnail"
        ? Promise.reject(new Error("offline"))
        : Promise.resolve(),
    );

    await expect(requestThumbnail("A", URL).promise).resolves.toBeNull();
  });
});
