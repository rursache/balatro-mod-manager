import { describe, expect, it } from "vitest";
import { sanitizeHtml } from "./sanitizeHtml";

describe("sanitizeHtml", () => {
  it("removes scripts, frames and event handlers", () => {
    const out = sanitizeHtml(
      '<p onclick="x()">hi</p><script>alert(1)</script><iframe src="https://a"></iframe><img src="https://a/b.png" onerror="x()">',
    );
    expect(out).toContain("<p>hi</p>");
    expect(out).toContain('<img src="https://a/b.png">');
    expect(out).not.toMatch(/script|iframe|onerror|onclick/);
  });

  it("drops script URLs but keeps normal links and image data", () => {
    const out = sanitizeHtml(
      '<a href="java\tscript:alert(1)">bad</a><a href="https://thunderstore.io">ok</a><img src="data:image/png;base64,AAAA"><a href="data:text/html,x">data</a>',
    );
    expect(out).toContain("<a>bad</a>");
    expect(out).toContain('<a href="https://thunderstore.io">ok</a>');
    expect(out).toContain('src="data:image/png;base64,AAAA"');
    expect(out).toContain("<a>data</a>");
  });

  it("keeps regular markdown output", () => {
    const html =
      '<h1>Title</h1><ul><li><input disabled="" type="checkbox"> task</li></ul><p align="center"><strong>bold</strong></p>';
    expect(sanitizeHtml(html)).toBe(html);
  });
});
