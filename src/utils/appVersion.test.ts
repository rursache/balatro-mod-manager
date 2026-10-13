import { describe, it, expect } from "vitest";
import {
  compareVersions,
  formatVersion,
  isReleaseVersion,
  isUpdateAvailable,
} from "./appVersion";

describe("isReleaseVersion", () => {
  it("accepts semver with or without a v prefix", () => {
    expect(isReleaseVersion("1.2.3")).toBe(true);
    expect(isReleaseVersion("v1.2.3")).toBe(true);
    expect(isReleaseVersion("1.2.3-beta.1")).toBe(true);
    expect(isReleaseVersion("1.2.3+build.5")).toBe(true);
  });

  it("rejects commit hashes, dev and partial versions", () => {
    expect(isReleaseVersion("aff48df")).toBe(false);
    expect(isReleaseVersion("1234567")).toBe(false);
    expect(isReleaseVersion("dev")).toBe(false);
    expect(isReleaseVersion("1.2")).toBe(false);
    expect(isReleaseVersion("")).toBe(false);
  });
});

describe("compareVersions", () => {
  it("orders by major, minor and patch numerically", () => {
    expect(compareVersions("1.2.3", "1.2.4")).toBe(-1);
    expect(compareVersions("1.10.0", "1.9.9")).toBe(1);
    expect(compareVersions("v2.0.0", "2.0.0")).toBe(0);
  });

  it("puts prereleases before their release", () => {
    expect(compareVersions("1.2.0-beta.1", "1.2.0")).toBe(-1);
    expect(compareVersions("1.2.0", "1.2.0-rc.1")).toBe(1);
    expect(compareVersions("1.2.0-beta.2", "1.2.0-beta.10")).toBe(-1);
    expect(compareVersions("1.2.0-alpha", "1.2.0-beta")).toBe(-1);
  });

  it("returns null when a side is not a version", () => {
    expect(compareVersions("aff48df", "1.0.0")).toBeNull();
    expect(compareVersions("1.0.0", "nightly")).toBeNull();
  });
});

describe("isUpdateAvailable", () => {
  it("offers newer releases to release builds", () => {
    expect(isUpdateAvailable("1.0.0", "v1.0.1")).toBe(true);
    expect(isUpdateAvailable("1.0.1", "v1.0.1")).toBe(false);
    expect(isUpdateAvailable("1.1.0", "v1.0.9")).toBe(false);
  });

  it("never nags hash or dev builds", () => {
    expect(isUpdateAvailable("aff48df", "v9.9.9")).toBe(false);
    expect(isUpdateAvailable("dev", "v9.9.9")).toBe(false);
  });

  it("ignores tags that are not versions", () => {
    expect(isUpdateAvailable("1.0.0", "latest")).toBe(false);
  });
});

describe("formatVersion", () => {
  it("prefixes releases with v and leaves other builds alone", () => {
    expect(formatVersion("1.2.3")).toBe("v1.2.3");
    expect(formatVersion("v1.2.3")).toBe("v1.2.3");
    expect(formatVersion("aff48df")).toBe("aff48df");
    expect(formatVersion("dev")).toBe("dev");
  });
});
