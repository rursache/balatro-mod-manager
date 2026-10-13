import { describe, it, expect } from "vitest";
import { isSupportedArchive } from "./archive";

describe("isSupportedArchive", () => {
  it("accepts supported extensions regardless of case", () => {
    for (const path of [
      "/mods/Mod.zip",
      "/mods/Mod.ZIP",
      "C:\\mods\\Mod.Zip",
      "Mod.tar",
      "Mod.TAR",
      "Mod.tar.gz",
      "Mod.TAR.GZ",
      "Mod.tgz",
      "Mod.TGZ",
      "Mod.rar",
      "Mod.RAR",
      "Mod.7z",
      "Mod.7Z",
    ]) {
      expect(isSupportedArchive(path), path).toBe(true);
    }
  });

  it("rejects unsupported files", () => {
    for (const path of ["Mod.gz", "Mod", "Mod.zip.txt", "Mod.7zip"]) {
      expect(isSupportedArchive(path), path).toBe(false);
    }
  });
});
