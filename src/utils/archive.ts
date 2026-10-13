export const SUPPORTED_ARCHIVE_EXTENSIONS = [
  ".zip",
  ".tar",
  ".tar.gz",
  ".tgz",
  ".rar",
  ".7z",
] as const;

export function isSupportedArchive(path: string): boolean {
  const lower = path.toLowerCase();
  return SUPPORTED_ARCHIVE_EXTENSIONS.some((ext) => lower.endsWith(ext));
}
