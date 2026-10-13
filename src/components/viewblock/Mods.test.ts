import { render, screen, within } from "@testing-library/svelte";
import { tick } from "svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Mods from "./Mods.svelte";
import { currentCategory, currentModView } from "../../stores/modStore";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  convertFileSrc: (path: string) => `asset://${path}`,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("$lib/stores", () => ({
  addMessage: vi.fn(),
}));

vi.mock("$lib/platform", () => ({
  isLinuxPlatform: vi.fn().mockResolvedValue(false),
}));

vi.mock("$lib/opener", () => ({
  openExternal: vi.fn(),
}));

function commandResult(command: string) {
  switch (command) {
    case "get_installed_mods_from_db":
    case "get_detected_local_mods":
    case "fetch_repo_mods":
      return [];
    case "get_cached_thumbnails_map":
      return {};
    case "mods_state_summary":
      return {
        installed: [],
        enabled: {},
        updates: {},
        thumbnails: {},
        descriptions: {},
      };
    default:
      return null;
  }
}

function renderMods() {
  return render(Mods, {
    props: { mod: null, handleDependencyCheck: vi.fn() },
  });
}

describe("Mods sidebar", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockImplementation(async (command: string) =>
      commandResult(command),
    );
    currentModView.set(null);
  });

  afterEach(() => {
    currentCategory.set("All Mods");
  });

  it("lists only search, All Mods and the categories on the All Mods tab", async () => {
    currentCategory.set("All Mods");
    renderMods();
    await tick();

    const sidebar = screen.getByTestId("catalog-sidebar");
    const labels = within(sidebar)
      .getAllByRole("button")
      .map((b) => b.textContent?.trim());
    expect(labels).toEqual([
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
    expect(labels).not.toContain("Installed Mods");
    expect(labels).not.toContain("Collections");
    expect(labels).not.toContain("Multiplayer");
  });

  it.each(["Installed Mods", "Multiplayer", "Collections"])(
    "shows %s full width without the category sidebar",
    async (category) => {
      currentCategory.set(category);
      renderMods();
      await tick();

      expect(screen.queryByTestId("catalog-sidebar")).toBeNull();
    },
  );

  it("brings the sidebar back when a catalog category is picked from outside", async () => {
    currentCategory.set("Installed Mods");
    renderMods();
    await tick();
    expect(screen.queryByTestId("catalog-sidebar")).toBeNull();

    currentCategory.set("Joker");
    await tick();

    const sidebar = screen.getByTestId("catalog-sidebar");
    expect(
      within(sidebar).getByRole("button", { name: "Joker" }).className,
    ).toContain("active");
  });
});
