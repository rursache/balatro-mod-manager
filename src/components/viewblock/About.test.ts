import { render, screen, fireEvent } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import About from "./About.svelte";
import { openExternal } from "$lib/opener";
import { get } from "svelte/store";
import { reportIssueStore } from "../../stores/modStore";

vi.mock("$lib/opener", () => ({
  openExternal: vi.fn(),
}));

describe("About view", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("opens the wiki page when wiki button is clicked", async () => {
    render(About);

    const wikiButton = screen.getByRole("button", { name: /visit wiki/i });
    await fireEvent.click(wikiButton);

    expect(openExternal).toHaveBeenCalledWith(
      "https://balatromods.miraheze.org/wiki/Main_Page",
    );
  });

  it("opens the fork repository when the GitHub button is clicked", async () => {
    render(About);

    await fireEvent.click(screen.getByRole("button", { name: /^github$/i }));

    expect(openExternal).toHaveBeenCalledWith(
      "https://github.com/rursache/balatro-mod-manager",
    );
  });

  it("credits the maintainer and the original creator", async () => {
    render(About);

    expect(screen.getByText(/maintained by/i)).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Radu Ursache" }));
    expect(openExternal).toHaveBeenCalledWith("https://radu.ursache.ro");

    expect(screen.getByText(/originally created by/i)).toBeTruthy();
    await fireEvent.click(
      screen.getByRole("button", { name: "Efe (Skyline)" }),
    );
    expect(openExternal).toHaveBeenCalledWith("https://github.com/skyline69");
  });

  it("shows the version it is given", () => {
    render(About, { props: { version: "1.2.3" } });

    expect(screen.getByText("Current version: v1.2.3")).toBeTruthy();
  });

  it("shows commit hash builds without a v prefix", () => {
    render(About, { props: { version: "aff48df" } });

    expect(screen.getByText("Current version: aff48df")).toBeTruthy();
  });

  it("hides the version line until the version is known", () => {
    render(About);

    expect(screen.queryByText(/current version/i)).toBeNull();
  });

  it("opens the report an issue dialog", async () => {
    reportIssueStore.set({ visible: false });
    render(About);

    await fireEvent.click(
      screen.getByRole("button", { name: /report an issue/i }),
    );

    expect(get(reportIssueStore).visible).toBe(true);
  });

  it("has no Ko-fi or status page links", () => {
    render(About);

    expect(screen.queryByRole("button", { name: /ko-fi/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /status page/i })).toBeNull();
  });
});
