import { describe, it, expect, vi, afterEach } from "vitest";
import { findScrollRoot, observeIntersection } from "./image-loader-shared";

function el(style = ""): HTMLDivElement {
  const div = document.createElement("div");
  if (style) div.setAttribute("style", style);
  return div;
}

describe("findScrollRoot", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("returns the nearest scrolling ancestor", () => {
    const outer = el("overflow-y: auto");
    const inner = el("overflow-y: scroll");
    const card = el();
    const img = el();
    outer.appendChild(inner);
    inner.appendChild(card);
    card.appendChild(img);
    document.body.appendChild(outer);

    expect(findScrollRoot(img)).toBe(inner);
  });

  it("falls back to the viewport when nothing scrolls", () => {
    const card = el();
    const img = el();
    card.appendChild(img);
    document.body.appendChild(card);

    expect(findScrollRoot(img)).toBeNull();
    expect(findScrollRoot(null)).toBeNull();
  });
});

describe("observeIntersection", () => {
  const original = globalThis.IntersectionObserver;

  afterEach(() => {
    globalThis.IntersectionObserver = original;
  });

  it("shares one observer per root and margin", () => {
    const created: IntersectionObserverInit[] = [];
    const observe = vi.fn();
    const unobserve = vi.fn();
    globalThis.IntersectionObserver = class {
      constructor(
        _cb: IntersectionObserverCallback,
        init: IntersectionObserverInit,
      ) {
        created.push(init);
      }
      observe = observe;
      unobserve = unobserve;
      disconnect() {}
    } as unknown as typeof IntersectionObserver;

    const root = el("overflow-y: auto");
    const a = el();
    const b = el();
    const c = el();
    const offA = observeIntersection(a, () => {}, {
      root,
      rootMargin: "360px 0px",
    });
    observeIntersection(b, () => {}, { root, rootMargin: "360px 0px" });
    observeIntersection(c, () => {}, { rootMargin: "360px 0px" });

    expect(created).toHaveLength(2);
    expect(created[0]).toMatchObject({ root, rootMargin: "360px 0px" });
    expect(created[1]).toMatchObject({ root: null, rootMargin: "360px 0px" });
    expect(observe).toHaveBeenCalledTimes(3);

    offA();
    expect(unobserve).toHaveBeenCalledWith(a);
  });
});
