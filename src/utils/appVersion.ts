// The app version comes from build.rs: a release tag like "1.2.3", a commit hash or "dev"

const SEMVER =
  /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

interface ParsedVersion {
  core: [number, number, number];
  pre: string[];
}

export function parseVersion(raw: string): ParsedVersion | null {
  const m = raw.trim().match(SEMVER);
  if (!m) return null;
  return {
    core: [Number(m[1]), Number(m[2]), Number(m[3])],
    pre: m[4] ? m[4].split(".") : [],
  };
}

/** True for release builds, false for commit hash or dev builds */
export function isReleaseVersion(raw: string): boolean {
  return parseVersion(raw) !== null;
}

function comparePre(a: string[], b: string[]): number {
  // A release sorts after its prereleases
  if (a.length === 0 || b.length === 0) return Math.sign(b.length - a.length);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    if (a[i] === undefined) return -1;
    if (b[i] === undefined) return 1;
    const an = /^\d+$/.test(a[i]);
    const bn = /^\d+$/.test(b[i]);
    if (an && bn) {
      const d = Number(a[i]) - Number(b[i]);
      if (d !== 0) return Math.sign(d);
    } else if (an !== bn) {
      return an ? -1 : 1;
    } else if (a[i] !== b[i]) {
      return a[i] < b[i] ? -1 : 1;
    }
  }
  return 0;
}

/** Semver ordering, null when either side is not a version */
export function compareVersions(a: string, b: string): number | null {
  const pa = parseVersion(a);
  const pb = parseVersion(b);
  if (!pa || !pb) return null;
  for (let i = 0; i < 3; i++) {
    const d = pa.core[i] - pb.core[i];
    if (d !== 0) return Math.sign(d);
  }
  return comparePre(pa.pre, pb.pre);
}

/** Hash and dev builds never nag, they are not comparable with releases */
export function isUpdateAvailable(current: string, latestTag: string): boolean {
  const cmp = compareVersions(current, latestTag);
  return cmp !== null && cmp < 0;
}

/** "v1.2.3" for releases, the hash or "dev" as is */
export function formatVersion(raw: string): string {
  const v = raw.trim().replace(/^v(?=\d)/i, "");
  return isReleaseVersion(v) ? `v${v}` : v;
}
