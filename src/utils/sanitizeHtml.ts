const BLOCKED_TAGS = new Set([
  "script",
  "style",
  "iframe",
  "frame",
  "frameset",
  "object",
  "embed",
  "applet",
  "link",
  "meta",
  "base",
  "form",
  "button",
  "textarea",
  "select",
  "option",
  "template",
  "noscript",
  "svg",
  "math",
]);

const URL_ATTRS = new Set([
  "href",
  "src",
  "xlink:href",
  "action",
  "formaction",
]);

function safeUrl(value: string, attr: string): boolean {
  // Browsers ignore whitespace and control characters inside a scheme
  const compact = Array.from(value)
    .filter((c) => c.charCodeAt(0) > 32)
    .join("")
    .toLowerCase();
  if (compact.startsWith("javascript:") || compact.startsWith("vbscript:")) {
    return false;
  }
  if (compact.startsWith("data:")) {
    return attr === "src" && /^data:image\/(png|gif|jpe?g|webp);/.test(compact);
  }
  return true;
}

/**
 * Strips scripts, event handlers and script URLs from rendered markdown. READMEs
 * come from Thunderstore uploads and the page runs with access to the app commands
 */
export function sanitizeHtml(html: string): string {
  if (!html) return "";
  if (typeof document === "undefined") return "";
  const template = document.createElement("template");
  template.innerHTML = html;
  const walk = (root: Element | DocumentFragment) => {
    for (const el of Array.from(root.children)) {
      const tag = el.tagName.toLowerCase();
      if (BLOCKED_TAGS.has(tag)) {
        el.remove();
        continue;
      }
      for (const attr of Array.from(el.attributes)) {
        const name = attr.name.toLowerCase();
        if (
          name.startsWith("on") ||
          name === "srcdoc" ||
          (URL_ATTRS.has(name) && !safeUrl(attr.value, name))
        ) {
          el.removeAttribute(attr.name);
        }
      }
      walk(el);
    }
  };
  walk(template.content);
  return template.innerHTML;
}
