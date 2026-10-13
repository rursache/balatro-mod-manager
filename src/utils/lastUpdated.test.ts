import { describe, it, expect } from "vitest";
import { formatLastUpdated, lastUpdatedDate } from "./lastUpdated";

describe("lastUpdatedDate", () => {
  it("is null when the index has no date", () => {
    expect(lastUpdatedDate(0)).toBeNull();
    expect(lastUpdatedDate(undefined)).toBeNull();
    expect(lastUpdatedDate(null)).toBeNull();
    expect(lastUpdatedDate(-5)).toBeNull();
    expect(lastUpdatedDate(Number.NaN)).toBeNull();
    expect(lastUpdatedDate("1760000000" as unknown as number)).toBeNull();
  });

  it("reads unix seconds from the index", () => {
    expect(lastUpdatedDate(1_760_000_000)?.toISOString()).toBe(
      "2025-10-09T08:53:20.000Z",
    );
  });

  it("accepts millisecond timestamps as is", () => {
    expect(lastUpdatedDate(1_760_000_000_000)?.toISOString()).toBe(
      "2025-10-09T08:53:20.000Z",
    );
  });
});

describe("formatLastUpdated", () => {
  it("hides missing dates instead of showing the epoch", () => {
    expect(formatLastUpdated(0)).toBeNull();
    expect(formatLastUpdated(undefined)).toBeNull();
  });

  it("formats a real date", () => {
    const label = formatLastUpdated(1_760_000_000, "en-US");
    expect(label).toMatch(/2025/);
    expect(label).toMatch(/Oct/);
    expect(label).not.toMatch(/1970/);
  });
});
