import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MultiplayerView from "./MultiplayerView.svelte";
import { openExternal } from "$lib/opener";
import { isModalOpen, resetModalStack } from "../../utils/modalStack";
import type { MultiplayerStatus, TrackPlan } from "../../utils/multiplayer";
import {
  manualLegacyCopy,
  modeFixture,
  statusFixture,
  trackFixture,
} from "../../test/multiplayerFixtures";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
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

if (!("animate" in Element.prototype)) {
  Object.defineProperty(Element.prototype, "animate", {
    value: () => ({
      cancel: () => {},
      play: () => {},
      pause: () => {},
      finished: Promise.resolve(),
    }),
    writable: true,
    configurable: true,
  });
}

const legacyInstalled = (): MultiplayerStatus =>
  statusFixture({
    active_track: "legacy",
    tracks: [
      trackFixture({
        track: "legacy",
        installed: true,
        official: true,
        installed_version: "0.5.7",
        tracked_name: "Multiplayer",
      }),
      trackFixture({ track: "api" }),
    ],
  });

const switchPlan: TrackPlan = {
  track: "api",
  packages: ["BalatroMultiplayer-MultiplayerAPI"],
  remove: [
    {
      role: "legacy",
      path: "/mods/BalatroMultiplayer-Multiplayer",
      folder: "BalatroMultiplayer-Multiplayer",
      tracked_name: "Multiplayer",
      replaced_by: null,
      reason: "legacy Multiplayer cannot be installed next to MultiplayerAPI",
    },
  ],
  replace: [],
  needs_confirmation: true,
  warnings: [],
};

function mockBackend(
  status: MultiplayerStatus,
  extra: Record<string, unknown> = {},
) {
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd in extra) return extra[cmd];
    switch (cmd) {
      case "multiplayer_status":
        return status;
      case "is_lovely_installed":
        return true;
      case "plan_catalog_install":
        return {
          mod_id: "ts:x",
          mod_title: "x",
          version: "1",
          lovely: null,
          steamodded: null,
          missing: [],
          warnings: [],
        };
      case "multiplayer_install":
        return {
          track: "api",
          installed: [],
          removed: ["Multiplayer"],
          plans: [],
          warnings: [],
        };
      default:
        return null;
    }
  });
}

async function renderView() {
  const result = render(MultiplayerView);
  await screen.findByRole("radiogroup", { name: /multiplayer track/i });
  return result;
}

function invoked(cmd: string) {
  return invokeMock.mock.calls.filter(([name]) => name === cmd);
}

describe("MultiplayerView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetModalStack();
  });

  afterEach(() => {
    resetModalStack();
  });

  it("says it is casual only and links ranked players to BET", async () => {
    mockBackend(statusFixture());
    await renderView();

    expect(screen.getByText("Casual and unranked play only")).toBeTruthy();
    expect(
      screen.getByText(
        "Ranked requires the official Balatro Essentials Toolkit (BET)",
      ),
    ).toBeTruthy();
    expect(
      screen.getByText(
        /reported to Balatro Mod Manager, not the\s+Multiplayer team/,
      ),
    ).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: /get bet/i }));
    expect(openExternal).toHaveBeenCalledWith(
      "https://new.balatromp.com/download",
    );
  });

  it("disables coming soon modes and enables published ones on the API track", async () => {
    mockBackend(statusFixture());
    await renderView();

    const modes = screen.getAllByRole("checkbox");
    const speedrun = modes.find((m) => m.textContent?.includes("Speedrun"));
    const pvp = modes.find((m) => m.textContent?.includes("PvP"));
    expect(pvp?.hasAttribute("disabled")).toBe(true);
    expect(pvp?.textContent).toContain("Coming soon");
    expect(speedrun?.hasAttribute("disabled")).toBe(false);

    await fireEvent.click(speedrun!);
    expect(speedrun?.getAttribute("aria-checked")).toBe("true");
  });

  it("disables every mode while the legacy track is selected", async () => {
    mockBackend(legacyInstalled());
    await renderView();

    for (const mode of screen.getAllByRole("checkbox")) {
      expect(mode.hasAttribute("disabled")).toBe(true);
    }
    expect(
      screen.getByText("Game modes need the Multiplayer API track"),
    ).toBeTruthy();
  });

  it("disables modes when Thunderstore is unavailable", async () => {
    mockBackend(
      statusFixture({ listing_available: false, listing_error: "offline" }),
    );
    await renderView();

    for (const mode of screen.getAllByRole("checkbox")) {
      expect(mode.hasAttribute("disabled")).toBe(true);
    }
    expect(screen.getByText(/Thunderstore is unavailable/)).toBeTruthy();
    expect(
      screen
        .getByRole("button", { name: /^install$/i })
        .hasAttribute("disabled"),
    ).toBe(true);
  });

  it("asks for confirmation before switching tracks", async () => {
    mockBackend(legacyInstalled(), { multiplayer_plan: switchPlan });
    await renderView();

    await fireEvent.click(
      screen.getByRole("radio", { name: /multiplayer api/i }),
    );
    await fireEvent.click(
      screen.getByRole("button", { name: /switch to this track/i }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("Switch to Multiplayer API?");
    expect(dialog.textContent).toContain("Multiplayer");
    expect(invoked("multiplayer_install")).toHaveLength(0);

    await fireEvent.click(
      screen.getByRole("button", { name: /switch tracks/i }),
    );

    await waitFor(() => expect(invoked("multiplayer_install")).toHaveLength(1));
    expect(invoked("multiplayer_install")[0][1]).toEqual({
      track: "api",
      gamemodes: [],
      confirmRemoval: true,
      installSteamodded: false,
    });
  });

  it("does not switch tracks when the confirmation is cancelled", async () => {
    mockBackend(legacyInstalled(), { multiplayer_plan: switchPlan });
    await renderView();

    await fireEvent.click(
      screen.getByRole("radio", { name: /multiplayer api/i }),
    );
    await fireEvent.click(
      screen.getByRole("button", { name: /switch to this track/i }),
    );
    await screen.findByRole("dialog");
    expect(isModalOpen()).toBe(true);
    await fireEvent.keyDown(window, { key: "Escape" });

    await waitFor(() => expect(isModalOpen()).toBe(false));
    expect(invoked("multiplayer_install")).toHaveLength(0);
  });

  it("installs a fresh track without a dialog", async () => {
    mockBackend(statusFixture(), {
      multiplayer_plan: {
        ...switchPlan,
        remove: [],
        needs_confirmation: false,
      },
    });
    await renderView();

    await fireEvent.click(screen.getByRole("button", { name: /^install$/i }));

    await waitFor(() => expect(invoked("multiplayer_install")).toHaveLength(1));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(invoked("multiplayer_install")[0][1]).toMatchObject({
      track: "api",
      confirmRemoval: false,
    });
  });

  it("offers the Thunderstore upgrade for a manual legacy install", async () => {
    mockBackend(
      statusFixture({
        active_track: "legacy",
        legacy_manual: manualLegacyCopy(),
        tracks: [
          trackFixture({
            track: "legacy",
            installed: true,
            official: false,
            installed_version: "0.5.4",
            update_available: true,
          }),
          trackFixture({ track: "api" }),
        ],
      }),
      {
        multiplayer_migrate_legacy: {
          track: "legacy",
          installed: [],
          removed: [],
          plans: [],
          warnings: [],
        },
      },
    );
    await renderView();

    const banner = screen.getByTestId("legacy-migration");
    expect(banner.textContent).toContain("v0.5.4");
    await fireEvent.click(
      screen.getByRole("button", {
        name: /upgrade to the thunderstore version/i,
      }),
    );
    await waitFor(() =>
      expect(invoked("multiplayer_migrate_legacy")).toHaveLength(1),
    );
  });

  it("hides the upgrade banner for the official package", async () => {
    mockBackend(legacyInstalled());
    await renderView();

    expect(screen.queryByTestId("legacy-migration")).toBeNull();
  });

  it("shows readiness checks with a Lovely row", async () => {
    mockBackend(
      statusFixture({
        active_track: "api",
        tracks: [
          trackFixture({ track: "legacy" }),
          trackFixture({ track: "api", installed: true, official: true }),
        ],
        readiness: [
          {
            id: "native_libs",
            label: "Bundled OpenSSL libraries",
            status: "ok",
            detail: null,
          },
          {
            id: "steam_running",
            label: "Steam is running",
            status: "warning",
            detail: "MultiplayerAPI needs Steam running to sign in",
          },
        ],
        gamemodes: [
          modeFixture({ installed: true, installed_version: "0.1.1" }),
        ],
      }),
      { is_lovely_installed: false },
    );
    await renderView();

    expect(screen.getByText("Bundled OpenSSL libraries")).toBeTruthy();
    expect(screen.getByText("Steam is running")).toBeTruthy();
    await waitFor(() =>
      expect(screen.getByText("Lovely is not installed")).toBeTruthy(),
    );
    expect(screen.getByRole("button", { name: /update lovely/i })).toBeTruthy();
  });
});
