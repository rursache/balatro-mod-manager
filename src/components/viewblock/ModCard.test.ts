import { render, screen } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import ModCard from "./ModCard.svelte";
import type { Mod, ThunderstoreInfo } from "../../stores/modStore";

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
