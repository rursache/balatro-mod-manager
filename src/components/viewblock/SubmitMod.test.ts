import { render, screen, fireEvent } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";
import SubmitMod from "./SubmitMod.svelte";
import { openExternal } from "$lib/opener";
import {
  INDEX_NEW_MOD_URL,
  INDEX_README_URL,
  INDEX_REPO_URL,
  INDEX_SCHEMA_URL,
} from "../../constants/links";

vi.mock("$lib/opener", () => ({
  openExternal: vi.fn(),
}));

describe("SubmitMod view", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it.each([
    [/read submission rules/i, INDEX_README_URL],
    [/view schema/i, INDEX_SCHEMA_URL],
    [/add mod files/i, INDEX_NEW_MOD_URL],
    [/view mod index repo/i, INDEX_REPO_URL],
  ])("button %s opens %s", async (name, url) => {
    render(SubmitMod);

    await fireEvent.click(screen.getByRole("button", { name }));

    expect(openExternal).toHaveBeenCalledWith(url);
  });

  it("points at the kasimeka mod index and never at the retired helper", async () => {
    render(SubmitMod);

    for (const button of screen.getAllByRole("button")) {
      await fireEvent.click(button);
    }

    const opened = vi.mocked(openExternal).mock.calls.map(([url]) => url);
    expect(opened.length).toBeGreaterThan(0);
    for (const url of opened) {
      expect(url).toContain("github.com/kasimeka/balatro-mod-index");
      expect(url).not.toContain("dasguney");
    }
  });
});
