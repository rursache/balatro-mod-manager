import { describe, it, expect } from "vitest";
import {
  ORPHAN_DESCRIPTION,
  computeInstallationStatus,
  findInstalledRow,
  indexInstalledRows,
  hasActionableUpdate,
  isBrowsableMod,
  isInstalledPlaceholder,
  reconcileInstalledPlaceholders,
} from "./installedPlaceholders";
import type { InstalledMod, Mod } from "../stores/modStore";

const catalogMod = (title: string, extra: Partial<Mod> = {}): Mod => ({
  title,
  description: `${title} description`,
  image: "",
  categories: [],
  colors: { color1: "", color2: "" },
  requires_steamodded: false,
  requires_talisman: false,
  publisher: "someone",
  repo: "",
  downloadURL: `index://${title}`,
  installed: false,
  last_updated: 1,
  _dirName: `${title}@someone`,
  ...extra,
});

const row = (
  name: string,
  orphaned = false,
  current_version: string | null = "1.0.0",
): InstalledMod => ({
  name,
  path: `/Mods/${name}`,
  orphaned,
  current_version,
  dependencies: ["Steamodded"],
});

const titles = (mods: Mod[]) => mods.map((m) => m.title);

describe("reconcileInstalledPlaceholders", () => {
  it("adds an orphaned placeholder for installed mods the index dropped", () => {
    const catalog = [catalogMod("Steamodded"), catalogMod("Saturn")];
    const installed = [
      row("Steamodded"),
      row("Saturn"),
      row("JokerDisplay", true, "d3c022d"),
      row("Multiplayer", true, "v0.5.4"),
    ];

    const out = reconcileInstalledPlaceholders(catalog, installed);

    expect(titles(out)).toEqual([
      "Steamodded",
      "Saturn",
      "JokerDisplay",
      "Multiplayer",
    ]);
    const joker = out.find((m) => m.title === "JokerDisplay")!;
    expect(isInstalledPlaceholder(joker)).toBe(true);
    expect(joker.orphaned).toBe(true);
    expect(joker.installedVersion).toBe("d3c022d");
    expect(joker.image).toBe("/images/cover.jpg");
    expect(joker._hasThumbnail).toBe(false);
    expect(joker.description).toBe(ORPHAN_DESCRIPTION);
    expect(joker.requires_steamodded).toBe(true);
    expect(joker._installedPath).toBe("/Mods/JokerDisplay");
  });

  it("does not duplicate mods the catalog lists under a slightly different name", () => {
    const catalog = [catalogMod("Divvy's Preview")];
    const out = reconcileInstalledPlaceholders(catalog, [
      row("Divvys Preview"),
    ]);
    expect(out).toBe(catalog);
  });

  it("returns the same array when nothing changed", () => {
    const installed = [row("Steamodded"), row("JokerDisplay", true)];
    const first = reconcileInstalledPlaceholders(
      [catalogMod("Steamodded")],
      installed,
    );
    expect(reconcileInstalledPlaceholders(first, installed)).toBe(first);
  });

  it("drops placeholders once the mod is uninstalled", () => {
    const withOrphan = reconcileInstalledPlaceholders(
      [catalogMod("Steamodded")],
      [row("Steamodded"), row("JokerDisplay", true)],
    );
    const out = reconcileInstalledPlaceholders(withOrphan, [row("Steamodded")]);
    expect(titles(out)).toEqual(["Steamodded"]);
  });

  it("drops a placeholder when the catalog lists the mod again", () => {
    const withPlaceholder = reconcileInstalledPlaceholders([], [row("Saturn")]);
    expect(isInstalledPlaceholder(withPlaceholder[0])).toBe(true);

    const out = reconcileInstalledPlaceholders(
      [...withPlaceholder, catalogMod("Saturn")],
      [row("Saturn")],
    );
    expect(out).toHaveLength(1);
    expect(isInstalledPlaceholder(out[0])).toBe(false);
  });

  it("syncs the orphan flag onto placeholders and stale catalog cards", () => {
    const placeholder = reconcileInstalledPlaceholders(
      [],
      [row("Multiplayer")],
    );
    expect(placeholder[0].orphaned).toBe(false);
    expect(placeholder[0].description).toBe("");

    const flagged = reconcileInstalledPlaceholders(placeholder, [
      row("Multiplayer", true, "v0.5.4"),
    ]);
    expect(flagged[0].orphaned).toBe(true);
    expect(flagged[0].description).toBe(ORPHAN_DESCRIPTION);
    expect(flagged[0].installedVersion).toBe("v0.5.4");

    const stale = reconcileInstalledPlaceholders(
      [catalogMod("JokerDisplay")],
      [row("JokerDisplay", true, "d3c022d")],
    );
    expect(stale[0].orphaned).toBe(true);
    expect(stale[0].installedVersion).toBe("d3c022d");
    expect(stale[0].description).toBe("JokerDisplay description");

    const cleared = reconcileInstalledPlaceholders(stale, [
      row("JokerDisplay"),
    ]);
    expect(cleared[0].orphaned).toBe(false);
    expect(cleared[0].installedVersion).toBeUndefined();
  });

  it("keeps a real cached description on placeholders", () => {
    const converted: Mod = {
      ...catalogMod("JokerDisplay"),
      _dirName: undefined,
      _installedPath: "/Mods/JokerDisplay",
    };
    const out = reconcileInstalledPlaceholders(
      [converted],
      [row("JokerDisplay", true)],
    );
    expect(out[0].orphaned).toBe(true);
    expect(out[0].description).toBe("JokerDisplay description");
  });
});

describe("isBrowsableMod", () => {
  it("hides orphans and placeholders from catalog browsing", () => {
    const [steamodded, joker, pending] = reconcileInstalledPlaceholders(
      [catalogMod("Steamodded"), catalogMod("JokerDisplay")],
      [row("Steamodded"), row("JokerDisplay", true), row("Pending")],
    );
    expect(isBrowsableMod(steamodded)).toBe(true);
    expect(isBrowsableMod(joker)).toBe(false);
    expect(isBrowsableMod(pending)).toBe(false);
  });

  it("keeps catalog cards hydrated without a dir name browsable", () => {
    expect(isBrowsableMod(catalogMod("Cached", { _dirName: undefined }))).toBe(
      true,
    );
  });
});

describe("Thunderstore rows", () => {
  const tsRow = (name: string, sourceId: string): InstalledMod => ({
    ...row(name),
    source: "thunderstore",
    source_id: sourceId,
  });
  const tsCard = (title: string, fullName: string, dir = `ts:${fullName}`) =>
    catalogMod(title, {
      _dirName: dir,
      source: dir.startsWith("ts:") ? "thunderstore" : "merged",
      thunderstore: {
        full_name: fullName,
      } as Mod["thunderstore"],
    });

  it("matches installed packages by id, never by a shared name", () => {
    const index = tsCard(
      "Pokermon",
      "InertSteak-Pokermon",
      "InertSteak@Pokermon",
    );
    const plus = tsCard("Pokermon (PokermonPlus)", "PokermonPlus-Pokermon");
    const status = computeInstallationStatus(
      [index, plus],
      [tsRow("Pokermon", "PokermonPlus-Pokermon")],
    );
    expect(status).toEqual({
      Pokermon: false,
      "Pokermon (PokermonPlus)": true,
    });
    expect(
      findInstalledRow(
        plus,
        indexInstalledRows([tsRow("Pokermon", "PokermonPlus-Pokermon")]),
      )?.name,
    ).toBe("Pokermon");
  });

  it("still matches index rows by name", () => {
    const status = computeInstallationStatus(
      [catalogMod("Divvy's Preview"), tsCard("Other", "a-Other")],
      [row("Divvys Preview")],
    );
    expect(status).toEqual({ "Divvy's Preview": true, Other: false });
  });

  it("gives packages the catalog lacks a placeholder card", () => {
    const catalog = [catalogMod("Steamodded")];
    const installed = [
      row("Steamodded"),
      tsRow("Multiplayer", "BalatroMultiplayer-Multiplayer"),
    ];
    const out = reconcileInstalledPlaceholders(catalog, installed);
    expect(titles(out)).toEqual(["Steamodded", "Multiplayer"]);
    const placeholder = out[1];
    expect(isInstalledPlaceholder(placeholder)).toBe(true);
    expect(computeInstallationStatus(out, installed)).toEqual({
      Steamodded: true,
      Multiplayer: true,
    });
  });

  it("keeps a Thunderstore card live for an index row the index dropped", () => {
    const card = tsCard("JokerDisplay", "nh6574-JokerDisplay");
    const installed = [row("JokerDisplay", true, "d3c022d")];
    const out = reconcileInstalledPlaceholders([card], installed);
    expect(out).toEqual([card]);
    expect(out[0].orphaned).toBeFalsy();
    expect(isBrowsableMod(out[0])).toBe(true);
    expect(computeInstallationStatus(out, installed)).toEqual({
      JokerDisplay: true,
    });
  });

  it("does not add a placeholder for a package the catalog lists", () => {
    const catalog = [
      tsCard("Pokermon (PokermonPlus)", "PokermonPlus-Pokermon"),
    ];
    const out = reconcileInstalledPlaceholders(catalog, [
      tsRow("Pokermon", "PokermonPlus-Pokermon"),
    ]);
    expect(out).toBe(catalog);
  });
});

describe("hasActionableUpdate", () => {
  it("ignores updates flagged for orphaned mods", () => {
    const mods = reconcileInstalledPlaceholders(
      [catalogMod("Steamodded")],
      [row("Steamodded"), row("Multiplayer", true)],
    );
    expect(hasActionableUpdate(mods, { Multiplayer: true })).toBe(false);
    expect(
      hasActionableUpdate(mods, { Multiplayer: true, Steamodded: true }),
    ).toBe(true);
    expect(hasActionableUpdate(mods, {})).toBe(false);
  });
});
