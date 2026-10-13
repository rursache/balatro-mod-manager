import { describe, it, expect } from "vitest";
import {
  recordedVersion,
  stripMarkdown,
  tagFromArchiveUrl,
  truncateText,
} from "./helpers";

describe("stripMarkdown", () => {
  describe("basic markdown removal", () => {
    it("should remove headers", () => {
      expect(stripMarkdown("# Header 1")).toBe("Header 1");
      expect(stripMarkdown("## Header 2")).toBe("Header 2");
      expect(stripMarkdown("###### Header 6")).toBe("Header 6");
    });

    it("should remove bold and italic markers", () => {
      expect(stripMarkdown("**bold**")).toBe("bold");
      expect(stripMarkdown("*italic*")).toBe("italic");
      expect(stripMarkdown("__bold__")).toBe("bold");
      expect(stripMarkdown("_italic_")).toBe("italic");
      expect(stripMarkdown("***bold italic***")).toBe("bold italic");
    });

    it("should remove links but keep text", () => {
      expect(stripMarkdown("[link text](https://example.com)")).toBe(
        "link text",
      );
      expect(stripMarkdown("[GitHub](https://github.com)")).toBe("GitHub");
    });

    it("should remove images but keep alt text", () => {
      // Note: current implementation leaves '!' prefix
      expect(stripMarkdown("![alt text](image.png)")).toBe("!alt text");
      expect(stripMarkdown("![screenshot](./img/screenshot.png)")).toBe(
        "!screenshot",
      );
    });

    it("should remove blockquotes", () => {
      expect(stripMarkdown("> quoted text")).toBe("quoted text");
    });

    it("should remove code blocks", () => {
      expect(stripMarkdown("```\ncode here\n```")).toBe("");
      expect(stripMarkdown("```javascript\nconst x = 1;\n```")).toBe("");
    });

    it("should remove inline code but keep content", () => {
      expect(stripMarkdown("use `npm install`")).toBe("use npm install");
    });

    it("should remove horizontal rules", () => {
      expect(stripMarkdown("---")).toBe("");
      expect(stripMarkdown("***")).toBe("");
      expect(stripMarkdown("___")).toBe("");
    });

    it("should remove list markers", () => {
      expect(stripMarkdown("- item")).toBe("item");
      expect(stripMarkdown("* item")).toBe("item");
    });
  });

  describe("HTML removal", () => {
    it("should remove HTML tags", () => {
      expect(stripMarkdown("<h1>Header</h1>")).toBe("Header");
      expect(stripMarkdown("<i>italic</i>")).toBe("italic");
      expect(stripMarkdown("<div>content</div>")).toBe("content");
    });

    it("should handle self-closing tags", () => {
      // Note: current implementation removes tags without adding space
      expect(stripMarkdown("text<br/>more")).toBe("textmore");
      expect(stripMarkdown("text<br />more")).toBe("textmore");
    });
  });

  describe("whitespace normalization", () => {
    it("should consolidate multiple newlines", () => {
      expect(stripMarkdown("line1\n\n\nline2")).toBe("line1 line2");
    });

    it("should replace consecutive whitespace with single space", () => {
      expect(stripMarkdown("word1    word2")).toBe("word1 word2");
    });

    it("should trim leading and trailing whitespace", () => {
      expect(stripMarkdown("  text  ")).toBe("text");
    });
  });

  describe("edge cases", () => {
    it("should return empty string for null/undefined", () => {
      expect(stripMarkdown(null as unknown as string)).toBe("");
      expect(stripMarkdown(undefined as unknown as string)).toBe("");
    });

    it("should return empty string for empty input", () => {
      expect(stripMarkdown("")).toBe("");
    });

    it("should handle plain text without changes", () => {
      expect(stripMarkdown("plain text")).toBe("plain text");
    });

    it("should handle complex mixed content", () => {
      const input =
        "# Title\n\n**Bold** and *italic* with [link](url)\n\n> Quote";
      const result = stripMarkdown(input);
      expect(result).toContain("Title");
      expect(result).toContain("Bold");
      expect(result).toContain("italic");
      expect(result).toContain("link");
      expect(result).toContain("Quote");
      expect(result).not.toContain("#");
      expect(result).not.toContain("**");
      expect(result).not.toContain("*");
      expect(result).not.toContain("[");
      expect(result).not.toContain(">");
    });
  });
});

describe("truncateText", () => {
  it("should not truncate short text", () => {
    expect(truncateText("short", 10)).toBe("short");
    expect(truncateText("exactly 10", 10)).toBe("exactly 10");
  });

  it("should truncate long text and add ellipsis", () => {
    expect(truncateText("this is a long text", 10)).toBe("this is a...");
  });

  it("should use default maxLength of 65", () => {
    const shortText = "a".repeat(65);
    const longText = "a".repeat(66);
    expect(truncateText(shortText)).toBe(shortText);
    expect(truncateText(longText)).toBe("a".repeat(65) + "...");
  });

  it("should trim whitespace before adding ellipsis", () => {
    expect(truncateText("hello world   extra", 12)).toBe("hello world...");
  });

  it("should return empty string for null/undefined", () => {
    expect(truncateText(null as unknown as string)).toBe("");
    expect(truncateText(undefined as unknown as string)).toBe("");
  });

  it("should return empty string for empty input", () => {
    expect(truncateText("")).toBe("");
  });
});

describe("recordedVersion", () => {
  const url =
    "https://github.com/Steamodded/smods/archive/refs/tags/26.829.0.zip";

  it("reads the tag from a tag archive URL", () => {
    expect(tagFromArchiveUrl(url)).toBe("26.829.0");
    expect(tagFromArchiveUrl("https://example.com/mod.zip")).toBeNull();
  });

  it("records the downloaded Steamodded tag instead of the catalog version", () => {
    expect(recordedVersion("Steamodded", url, "26.1002.0")).toBe("26.829.0");
  });

  it("keeps the catalog version for other mods", () => {
    expect(recordedVersion("Saturn", url, "v1.2")).toBe("v1.2");
    expect(recordedVersion("Saturn", url, undefined)).toBe("");
  });
});
