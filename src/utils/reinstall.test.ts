import { describe, expect, it } from "vitest";
import type { InstalledMod, Mod } from "../stores/modStore";
import {
  hasSourceSwitch,
  indexDownloadUrl,
  reinstallTarget,
  specialInstaller,
} from "./reinstall";

const base: Mod = {
  title: "Some Mod",
  description: "",
  image: "",
  categories: [],
  colors: { color1: "#000", color2: "#111" },
  requires_steamodded: false,
  requires_talisman: false,
  publisher: "pub",
  repo: "",
  downloadURL: "https://example.com/some.zip",
  installed: true,
  last_updated: 0,
  _dirName: "pub@SomeMod",
  source: "index",
  version: "1.2.0",
};

const thunderstore = {
  namespace: "nh6574",
  name: "JokerDisplay",
  full_name: "nh6574-JokerDisplay",
} as Mod["thunderstore"];

const steamodded: Mod = {
  ...base,
  title: "Steamodded",
  _dirName: "Steamodded@smods",
  source: "merged",
  thunderstore: {
    ...thunderstore,
    namespace: "Steamodded",
    name: "Steamodded",
    full_name: "Steamodded-Steamodded",
  } as Mod["thunderstore"],
};

function row(fields: Partial<InstalledMod>): InstalledMod {
  return {
    name: "Some Mod",
    path: "/Mods/SomeMod",
    source: "index",
    ...fields,
  };
}

describe("source switch", () => {
  it("is never offered for Steamodded even when merged", () => {
    expect(specialInstaller(steamodded)).toBe("steamodded");
    expect(hasSourceSwitch(steamodded)).toBe(false);
  });

  it("is never offered for Talisman", () => {
    const talisman: Mod = {
      ...base,
      title: "Talisman",
      source: "merged",
      thunderstore,
    };
    expect(hasSourceSwitch(talisman)).toBe(false);
  });

  it("is offered for other merged mods only", () => {
    expect(hasSourceSwitch({ ...base, source: "merged", thunderstore })).toBe(
      true,
    );
    expect(hasSourceSwitch({ ...base, source: "merged" })).toBe(false);
    expect(
      hasSourceSwitch({ ...base, source: "thunderstore", thunderstore }),
    ).toBe(false);
    expect(hasSourceSwitch(base)).toBe(false);
  });
});

describe("reinstallTarget", () => {
  it("is null when the mod is not installed", () => {
    expect(reinstallTarget(base, null)).toBeNull();
  });

  it("reinstalls the installed Thunderstore version", () => {
    const mod: Mod = {
      ...base,
      title: "JokerDisplay",
      _dirName: "ts:nh6574-JokerDisplay",
      source: "thunderstore",
      thunderstore,
    };
    const tracked = row({
      name: "JokerDisplay",
      source: "thunderstore",
      source_id: "nh6574-JokerDisplay",
      current_version: "2.0.5",
    });
    expect(reinstallTarget(mod, tracked)).toEqual({
      kind: "thunderstore",
      row: tracked,
      version: "2.0.5",
    });
  });

  it("needs a recorded version for Thunderstore rows", () => {
    const mod: Mod = { ...base, source: "merged", thunderstore };
    expect(
      reinstallTarget(
        mod,
        row({ source: "thunderstore", current_version: "" }),
      ),
    ).toBeNull();
  });

  it("reinstalls index rows of merged mods from the index download", () => {
    const mod: Mod = { ...base, source: "merged", thunderstore };
    const target = reinstallTarget(mod, row({ current_version: "1.1.0" }));
    expect(target).toMatchObject({
      kind: "index",
      version: "1.2.0",
      url: "https://example.com/some.zip",
    });
  });

  it("falls back to the index URL scheme without a download URL", () => {
    expect(indexDownloadUrl({ ...base, downloadURL: "" })).toBe(
      "index://pub@SomeMod",
    );
  });

  it("is not offered for local or manual installs", () => {
    const placeholder: Mod = {
      ...base,
      _dirName: undefined,
      _installedPath: "/Mods/Local",
    };
    expect(reinstallTarget(placeholder, row({}))).toBeNull();
  });

  it("reinstalls the picked Steamodded tag, the installed one by default", () => {
    const tracked = row({ name: "Steamodded", current_version: "26.1002.0" });
    expect(reinstallTarget(steamodded, tracked)).toMatchObject({
      kind: "steamodded",
      version: "26.1002.0",
    });
    expect(reinstallTarget(steamodded, tracked, "26.829.0")).toMatchObject({
      kind: "steamodded",
      version: "26.829.0",
    });
    expect(reinstallTarget(steamodded, tracked, "newest")).toMatchObject({
      version: "26.1002.0",
    });
  });

  it("is null for Steamodded without a known tag", () => {
    expect(reinstallTarget(steamodded, row({ name: "Steamodded" }))).toBeNull();
  });
});
