import { fireEvent, render, screen } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ModCard from "./ModCard.svelte";
import {
  installationStatus,
  updateAvailableStore,
  type Mod,
  type ThunderstoreInfo,
} from "../../stores/modStore";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
  convertFileSrc: (p: string) => `asset://${p}`,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@tauri-apps/api/path", () => ({
  configDir: vi.fn().mockResolvedValue("/config"),
  join: (...parts: string[]) => Promise.resolve(parts.join("/")),
}));

vi.mock("$lib/platform", () => ({
  isLinuxPlatform: vi.fn().mockResolvedValue(false),
}));

const ts = (extra: Partial<ThunderstoreInfo> = {}): ThunderstoreInfo => ({
  namespace: "nh6574",
  name: "JokerDisplay",
  full_name: "nh6574-JokerDisplay",
  owner: "nh6574",
  package_url: "https://thunderstore.io/c/balatro/p/nh6574/JokerDisplay/",
  latest_version: "1.8.4",
  download_url: "",
  website_url: "",
  icon_url: "",
  downloads_total: 12345,
  rating: 3,
  deprecated: false,
  nsfw: false,
  ai_generated: false,
  categories_raw: [],
  date_updated: "",
  description: "",
  ...extra,
});

const mod = (extra: Partial<Mod> = {}): Mod => ({
  title: "JokerDisplay",
  description: "Shows joker info",
  image: "/images/cover.jpg",
  categories: [],
  colors: { color1: "#000", color2: "#111" },
  requires_steamodded: false,
  requires_talisman: false,
  publisher: "nh6574",
  repo: "",
  downloadURL: "",
  installed: false,
  last_updated: 0,
  ...extra,
});

const badgeTexts = () =>
  Array.from(
    screen.queryByTestId("mod-badges")?.querySelectorAll(".badge") ?? [],
  ).map((b) => b.textContent?.trim());

describe("ModCard badges", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
  });

  it("shows no badges for plain index mods", () => {
    render(ModCard, { mod: mod({ source: "index" }) });
    expect(screen.queryByTestId("mod-badges")).toBeNull();
  });

  it("shows both sources for merged mods with owner and downloads", () => {
    render(ModCard, {
      mod: mod({ source: "merged", thunderstore: ts(), downloads: 12345 }),
    });
    expect(badgeTexts()).toEqual(["Index", "Thunderstore"]);
    expect(screen.getByText("nh6574")).toBeTruthy();
    expect(screen.getByText("12.3K")).toBeTruthy();
  });

  it("flags deprecated, AI generated and NSFW packages", () => {
    render(ModCard, {
      mod: mod({
        source: "thunderstore",
        thunderstore: ts({ deprecated: true, ai_generated: true, nsfw: true }),
        deprecated: true,
        aiGenerated: true,
        nsfw: true,
        downloads: 7,
      }),
    });
    expect(badgeTexts()).toEqual(["Thunderstore", "Deprecated", "AI", "NSFW"]);
    expect(screen.getByText("7")).toBeTruthy();
  });

  it("omits the download count for index only mods", () => {
    render(ModCard, { mod: mod({ source: "index", downloads: null }) });
    expect(document.querySelector(".meta-downloads")).toBeNull();
    expect(document.querySelector(".meta-owner")?.textContent).toContain(
      "nh6574",
    );
  });
});

const placeholder = (extra: Partial<Mod> = {}): Mod =>
  mod({
    title: "Multiplayer",
    description: "",
    publisher: "BalatroMultiplayer",
    installed: true,
    installedVersion: "0.5.7",
    _installedPath: "/Mods/BalatroMultiplayer-Multiplayer",
    ...extra,
  });

const multiplayer = (updateAvailable = false) => ({
  role: "legacy" as const,
  fullName: "BalatroMultiplayer-Multiplayer",
  latestVersion: "0.5.8",
  updateAvailable,
});

describe("ModCard installed placeholders", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(null);
    installationStatus.set({ Multiplayer: true });
    updateAvailableStore.set({});
  });

  it("shows a fallback instead of an endless skeleton without a description", () => {
    render(ModCard, { mod: placeholder() });
    expect(document.querySelector(".desc-skeleton")).toBeNull();
    expect(screen.getByText("No description available")).toBeTruthy();
  });

  it("keeps the skeleton for catalog cards still loading their description", () => {
    render(ModCard, { mod: mod({ description: "", _dirName: "jd@nh" }) });
    expect(document.querySelector(".desc-skeleton")).not.toBeNull();
  });

  it("labels Multiplayer packages with owner, version and badge", () => {
    render(ModCard, {
      mod: placeholder({
        description: "Legacy standalone Balatro Multiplayer mod",
        multiplayer: multiplayer(),
      }),
    });
    expect(badgeTexts()).toEqual(["Multiplayer"]);
    expect(document.querySelector(".meta-owner")?.textContent).toContain(
      "BalatroMultiplayer",
    );
    expect(document.querySelector(".tag.version")?.textContent).toContain(
      "0.5.7",
    );
    expect(document.querySelector(".tag.orphaned")).toBeNull();
    expect(document.querySelector(".collection-button")).toBeNull();
  });

  it("shows the Multiplayer tab update state and opens that tab to update", async () => {
    // A stale summary flag does not matter, the Multiplayer status decides
    updateAvailableStore.set({ Multiplayer: false });
    const onmodclick = vi.fn();
    const oninstallclick = vi.fn();
    render(ModCard, {
      mod: placeholder({ multiplayer: multiplayer(true) }),
      onmodclick,
      oninstallclick,
    });
    const update = screen.getByText("Update").closest("button")!;
    await fireEvent.click(update);
    expect(onmodclick).toHaveBeenCalledTimes(1);
    expect(oninstallclick).not.toHaveBeenCalled();
  });

  it("hides Update when the Multiplayer tab reports none", () => {
    updateAvailableStore.set({ Multiplayer: true });
    render(ModCard, { mod: placeholder({ multiplayer: multiplayer(false) }) });
    expect(screen.queryByText("Update")).toBeNull();
  });
});
