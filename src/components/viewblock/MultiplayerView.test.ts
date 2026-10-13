import { fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import MultiplayerView from "./MultiplayerView.svelte";
import { openExternal } from "$lib/opener";
import { isModalOpen, resetModalStack } from "../../utils/modalStack";
import { get } from "svelte/store";
import {
  multiplayerStatusStore,
  multiplayerUninstallRequest,
} from "../../stores/multiplayerStatus";
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

  it("keeps the casual badge in the header and the ranked notice under Readiness", async () => {
    mockBackend(statusFixture());
    const { container } = await renderView();

    const header = container.querySelector("header.mp-header")!;
    expect(header.textContent).toContain("Casual and unranked play only");
    expect(header.textContent).not.toContain("Balatro Essentials Toolkit");
    expect(header.textContent).not.toContain("reported to Balatro Mod Manager");

    const readiness = screen.getByRole("heading", { name: "Readiness" });
    const footer = container.querySelector("footer.mp-footer")!;
    expect(footer.querySelector(".ranked-notice")).not.toBeNull();
    expect(footer.querySelector(".support-note")).not.toBeNull();
    expect(readiness.closest("section")!.nextElementSibling === footer).toBe(
      true,
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

  it("shares its status with the Installed cards", async () => {
    const status = legacyInstalled();
    mockBackend(status);
    await renderView();
    expect(get(multiplayerStatusStore)).toEqual(status);
  });

  it("opens the uninstall dialog an Installed card asked for", async () => {
    mockBackend(legacyInstalled(), {
      multiplayer_uninstall: { track: "legacy", removed: [], removed_dlls: [] },
    });
    multiplayerUninstallRequest.set({ track: "legacy" });
    await renderView();

    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("Uninstall Multiplayer?");
    expect(get(multiplayerUninstallRequest)).toBeNull();
    // Nothing is removed until the user confirms
    expect(invoked("multiplayer_uninstall")).toHaveLength(0);
  });

  it("drops an uninstall request for a track that is not installed", async () => {
    mockBackend(legacyInstalled());
    multiplayerUninstallRequest.set({ track: "api" });
    await renderView();

    await waitFor(() => expect(get(multiplayerUninstallRequest)).toBeNull());
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("lists exactly what uninstalling the API removes and keeps", async () => {
    const removal = (folder: string, role: "api" | "gamemode") => ({
      role,
      path: `/mods/${folder}`,
      folder,
      tracked_name: null,
      replaced_by: null,
      reason: "uninstall",
    });
    mockBackend(
      statusFixture({
        active_track: "api",
        tracks: [
          trackFixture({ track: "legacy" }),
          trackFixture({
            track: "api",
            installed: true,
            official: true,
            installed_version: "0.1.2",
          }),
        ],
        uninstall: [
          { track: "legacy", remove: [], keep: [] },
          {
            track: "api",
            remove: [
              removal("BalatroMultiplayer-MultiplayerAPI", "api"),
              removal("BalatroMultiplayer-MultiplayerSpeedrun", "gamemode"),
            ],
            keep: [
              {
                ...removal("MyMPMode", "gamemode"),
                reason:
                  "it is not an official game mode and stops loading without MultiplayerAPI",
              },
            ],
          },
        ],
      }),
      {
        multiplayer_uninstall: { track: "api", removed: [], removed_dlls: [] },
      },
    );
    await renderView();

    await fireEvent.click(screen.getByRole("button", { name: /^uninstall$/i }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("BalatroMultiplayer-MultiplayerAPI");
    expect(dialog.textContent).toContain(
      "BalatroMultiplayer-MultiplayerSpeedrun",
    );
    expect(dialog.textContent).toContain("These stay in the Mods folder");
    expect(dialog.textContent).toContain("MyMPMode");

    const buttons = screen.getAllByRole("button", { name: /^uninstall$/i });
    await fireEvent.click(buttons[buttons.length - 1]);
    await waitFor(() =>
      expect(invoked("multiplayer_uninstall")).toHaveLength(1),
    );
    expect(invoked("multiplayer_uninstall")[0][1]).toEqual({ track: "api" });
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
