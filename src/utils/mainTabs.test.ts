import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CATALOG_CATEGORIES,
  MAIN_TABS,
  categoryForTab,
  chooseStartupTab,
  isModsTab,
  resolveStartupTab,
  showsPagination,
  tabForCategory,
} from "./mainTabs";

describe("main tabs", () => {
  it("lists the top tabs in order with Installed Mods first", () => {
    expect(MAIN_TABS.map((t) => t.label)).toEqual([
      "Installed Mods",
      "All Mods",
      "Multiplayer",
      "Collections",
      "Backups",
      "Settings",
      "About",
    ]);
  });

  it("keeps Installed Mods, Collections and Multiplayer out of the catalog sidebar", () => {
    expect(CATALOG_CATEGORIES).toEqual([
      "Search",
      "All Mods",
      "Content",
      "Miscellaneous",
      "Joker",
      "Quality of Life",
      "Technical",
      "Resource Packs",
      "API",
    ]);
  });

  it("maps categories to the tab that shows them", () => {
    expect(tabForCategory("Installed Mods")).toBe("installed");
    expect(tabForCategory("Multiplayer")).toBe("multiplayer");
    expect(tabForCategory("Collections")).toBe("collections");
    for (const category of CATALOG_CATEGORIES) {
      expect(tabForCategory(category)).toBe("catalog");
    }
  });

  it("reopens the last catalog category on the All Mods tab", () => {
    expect(categoryForTab("catalog", "Joker")).toBe("Joker");
    expect(categoryForTab("catalog", "Search")).toBe("Search");
    expect(categoryForTab("catalog", "Installed Mods")).toBe("All Mods");
    expect(categoryForTab("catalog", "Popular")).toBe("All Mods");
    expect(categoryForTab("installed", "Joker")).toBe("Installed Mods");
    expect(categoryForTab("multiplayer", "Joker")).toBe("Multiplayer");
    expect(categoryForTab("collections", "Joker")).toBe("Collections");
  });

  it("only treats the four mod views as Mods tabs", () => {
    expect(MAIN_TABS.filter((t) => isModsTab(t.id)).map((t) => t.id)).toEqual([
      "installed",
      "catalog",
      "multiplayer",
      "collections",
    ]);
  });

  it("shows pagination only for the paged catalog grid", () => {
    expect(showsPagination("All Mods")).toBe(true);
    expect(showsPagination("Joker")).toBe(true);
    expect(showsPagination("Search")).toBe(false);
    expect(showsPagination("Installed Mods")).toBe(false);
    expect(showsPagination("Multiplayer")).toBe(false);
    expect(showsPagination("Collections")).toBe(false);
  });
});

describe("startup tab", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("opens Installed Mods when anything is installed", () => {
    expect(chooseStartupTab(1, 0)).toBe("installed");
    expect(chooseStartupTab(0, 2)).toBe("installed");
    expect(chooseStartupTab(0, 0)).toBe("catalog");
  });

  it("skips the local scan when a tracked mod exists", async () => {
    const local = vi.fn().mockResolvedValue([]);
    const tab = await resolveStartupTab({
      tracked: async () => [{ name: "Steamodded" }],
      local,
    });
    expect(tab).toBe("installed");
    expect(local).not.toHaveBeenCalled();
  });

  it("counts detected local mods when nothing is tracked", async () => {
    const tab = await resolveStartupTab({
      tracked: async () => [],
      local: async () => [{ name: "Manual" }],
    });
    expect(tab).toBe("installed");
  });

  it("opens All Mods when nothing is installed or the lists fail", async () => {
    expect(
      await resolveStartupTab({
        tracked: async () => [],
        local: async () => [],
      }),
    ).toBe("catalog");
    expect(
      await resolveStartupTab({
        tracked: () => Promise.reject(new Error("db locked")),
        local: () => Promise.reject(new Error("no mods folder")),
      }),
    ).toBe("catalog");
  });

  it("falls back to All Mods when the installed list hangs", async () => {
    vi.useFakeTimers();
    const pending = resolveStartupTab({
      tracked: () => new Promise(() => {}),
      local: async () => [],
      timeoutMs: 500,
    });
    await vi.advanceTimersByTimeAsync(500);
    await expect(pending).resolves.toBe("catalog");
  });
});
