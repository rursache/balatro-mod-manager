import { render, screen } from "@testing-library/svelte";
import { describe, it, expect } from "vitest";
import LastUpdated from "./LastUpdated.svelte";

describe("LastUpdated", () => {
  it.each([0, undefined, null])("renders nothing for %s", (value) => {
    const { container } = render(LastUpdated, { props: { value } });
    expect(screen.queryByTestId("last-updated")).toBeNull();
    expect(container.textContent?.trim()).toBe("");
  });

  it("shows the date when the index has one", () => {
    render(LastUpdated, {
      props: { value: 1_760_000_000, class: "tag updated" },
    });
    const label = screen.getByTestId("last-updated");
    expect(label.textContent).toMatch(/2025/);
    expect(label.textContent).not.toMatch(/1970/);
    expect(label.classList.contains("updated")).toBe(true);
    expect(label.getAttribute("title")).toBe("Last updated");
  });
});
