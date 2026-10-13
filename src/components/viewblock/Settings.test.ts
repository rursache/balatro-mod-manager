import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { get } from "svelte/store";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import Settings from "./Settings.svelte";
import {
  cachedVersions,
  catalogLastRefreshed,
  currentJokerView,
  currentModView,
  modsStore,
  searchResults,
} from "../../stores/modStore";
import { descriptionsStore } from "../../stores/descriptions";
import {
  hideAiGeneratedMods,
  showDeprecatedMods,
  showNsfwMods,
  sourceFilter,
  thunderstoreEnabled,
} from "../../stores/catalogFilters";

const invokeMock = vi.hoisted(() => vi.fn());
const addMessageMock = vi.hoisted(() => vi.fn());
const isFullscreenMock = vi.hoisted(() => vi.fn());
const setFullscreenMock = vi.hoisted(() => vi.fn());
const onResizedMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("$lib/stores", () => ({
  addMessage: (...args: unknown[]) => addMessageMock(...args),
}));

vi.mock("@tauri-apps/plugin-os", () => ({
  platform: vi.fn().mockResolvedValue("windows"),
}));

vi.mock("@tauri-apps/api/window", () => ({
  Window: {
    getCurrent: () => ({
      isFullscreen: (...args: unknown[]) => isFullscreenMock(...args),
      setFullscreen: (...args: unknown[]) => setFullscreenMock(...args),
      onResized: (...args: unknown[]) => onResizedMock(...args),
    }),
  },
}));

function createMockMod(title: string) {
  return {
    title,
    description: "desc",
    image: "img",
    categories: [],
    colors: { color1: "#000", color2: "#111" },
    requires_steamodded: false,
    requires_talisman: false,
    publisher: "pub",
    repo: "repo",
    downloadURL: "https://example.com",
    installed: true,
    last_updated: 0,
  };
}

describe("Settings view", () => {
  let mockStorage: Record<string, string> = {};

  beforeEach(() => {
    vi.clearAllMocks();
    isFullscreenMock.mockResolvedValue(true);
    setFullscreenMock.mockResolvedValue(undefined);
    onResizedMock.mockResolvedValue(() => {});
    mockStorage = {};
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => mockStorage[key] ?? null,
      setItem: (key: string, value: string) => {
        mockStorage[key] = value;
      },
      removeItem: (key: string) => {
        delete mockStorage[key];
      },
      clear: () => {
        mockStorage = {};
      },
      key: (i: number) => Object.keys(mockStorage)[i] ?? null,
      get length() {
        return Object.keys(mockStorage).length;
      },
    });
    modsStore.set([createMockMod("mod-a")]);
    searchResults.set([createMockMod("mod-b")]);
    currentModView.set(createMockMod("mod-c"));
    currentJokerView.set(createMockMod("mod-d"));
    catalogLastRefreshed.set(12345);
    cachedVersions.set({ steamodded: ["1.0.0"], talisman: ["2.0.0"] });
    descriptionsStore.set({ "mod-a": "cached description" });

    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "get_all_settings") {
        return {
          discord_rpc: false,
          lovely_console: false,
          background_enabled: false,
          compat_helper: true,
          linux_prefix: "",
          launch_mode: "modded",
        };
      }
      if (command === "get_balatro_path") {
        return "";
      }
      if (command === "get_mods_folder") {
        return "/tmp/mods";
      }
      if (command === "open_directory") {
        return args;
      }
      if (command === "reindex_mods") {
        return [0, 4];
      }
      if (command === "clear_cache") {
        return null;
      }
      return null;
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("opens the mods folder from Settings", async () => {
    render(Settings);

    await fireEvent.click(
      screen.getByRole("button", { name: /open mods folder/i }),
    );

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_mods_folder");
      expect(invokeMock).toHaveBeenCalledWith("open_directory", {
        path: "/tmp/mods",
      });
    });
  });

  it("reindexes mod database and reports success", async () => {
    render(Settings);

    await fireEvent.click(
      screen.getByRole("button", { name: /validate mod database/i }),
    );

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("reindex_mods");
      expect(addMessageMock).toHaveBeenCalledWith(
        "Reindex complete: Cleaned 4 database entries",
        "success",
      );
    });
  });

  it("clears caches and resets related in-memory stores", async () => {
    render(Settings);

    await fireEvent.click(screen.getByRole("button", { name: /clear cache/i }));

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("clear_cache");
      expect(addMessageMock).toHaveBeenCalledWith(
        "Successfully cleared all caches!",
        "success",
      );
    });

    expect(get(modsStore)).toEqual([]);
    expect(get(searchResults)).toEqual([]);
    expect(get(currentModView)).toBeNull();
    expect(get(currentJokerView)).toBeNull();
    expect(get(catalogLastRefreshed)).toBeNull();
    expect(get(cachedVersions)).toEqual({ steamodded: [], talisman: [] });
    expect(get(descriptionsStore)).toEqual({});
  });

  it("toggles and persists the catalog filters", async () => {
    sourceFilter.set("thunderstore");
    render(Settings);

    const ts = screen.getByRole("checkbox", { name: "Thunderstore Mods" });
    const nsfw = screen.getByRole("checkbox", { name: "Show NSFW Mods" });
    const ai = screen.getByRole("checkbox", { name: "Hide AI Generated Mods" });
    const deprecated = screen.getByRole("checkbox", {
      name: "Show Deprecated Mods",
    });
    expect((ts as HTMLInputElement).checked).toBe(true);
    expect((nsfw as HTMLInputElement).checked).toBe(false);
    expect((ai as HTMLInputElement).checked).toBe(false);
    expect((deprecated as HTMLInputElement).checked).toBe(false);

    await fireEvent.click(nsfw);
    await fireEvent.click(ai);
    await fireEvent.click(deprecated);
    await fireEvent.click(ts);

    expect(get(showNsfwMods)).toBe(true);
    expect(get(hideAiGeneratedMods)).toBe(true);
    expect(get(showDeprecatedMods)).toBe(true);
    expect(get(thunderstoreEnabled)).toBe(false);
    expect(get(sourceFilter)).toBe("all");
    expect(mockStorage["catalog.showNsfw"]).toBe("true");
    expect(mockStorage["catalog.hideAiGenerated"]).toBe("true");
    expect(mockStorage["catalog.showDeprecated"]).toBe("true");
    expect(mockStorage["catalog.thunderstoreEnabled"]).toBe("false");
    expect(invokeMock).toHaveBeenCalledWith("set_thunderstore_enabled", {
      enabled: false,
    });

    for (const store of [
      showNsfwMods,
      hideAiGeneratedMods,
      showDeprecatedMods,
    ]) {
      store.set(false);
    }
    thunderstoreEnabled.set(true);
  });

  it("has no analytics or privacy toggle", async () => {
    render(Settings);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("get_all_settings");
    });
    expect(screen.queryByText(/anonymous usage data/i)).toBeNull();
    expect(screen.queryByRole("heading", { name: /privacy/i })).toBeNull();
    expect(invokeMock).not.toHaveBeenCalledWith(
      "set_analytics_status",
      expect.anything(),
    );
  });

  it("shows and toggles fullscreen state", async () => {
    render(Settings);

    const toggle = await screen.findByRole("checkbox", {
      name: /fullscreen/i,
    });
    await waitFor(() => {
      expect((toggle as HTMLInputElement).checked).toBe(true);
    });

    await fireEvent.click(toggle);

    await waitFor(() => {
      expect(setFullscreenMock).toHaveBeenCalledWith(false);
    });
    expect((toggle as HTMLInputElement).checked).toBe(false);
  });

  it("resyncs fullscreen state when the window is resized", async () => {
    render(Settings);

    const toggle = await screen.findByRole("checkbox", {
      name: /fullscreen/i,
    });
    await waitFor(() => {
      expect((toggle as HTMLInputElement).checked).toBe(true);
      expect(onResizedMock).toHaveBeenCalled();
    });

    isFullscreenMock.mockResolvedValue(false);
    const onResized = onResizedMock.mock.calls[0][0] as () => void;
    onResized();

    await waitFor(() => {
      expect((toggle as HTMLInputElement).checked).toBe(false);
    });
  });
});
