import { cleanup, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ICON_NAMES } from "../lib/Icon.svelte";
import Detail from "./Detail.svelte";

function stubItem(id: string) {
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>().mockImplementation(() =>
      Promise.resolve(
        new Response(
          JSON.stringify({
            id,
            name: "見本項目",
            summary: "s",
            status: "stable",
            updated_at: "2026-08-07",
            body: "b",
          }),
          { status: 200 },
        ),
      ),
    ),
  );
}

describe("Detail", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
  });

  // Independent expectation: the published dictionary as of this test.
  // Not derived from ICON_NAMES, so a dictionary regression cannot
  // silently re-green this list.
  const PUBLISHED_ICONS = [
    "menu",
    "x",
    "sun",
    "moon",
    "monitor",
    "chevron-left",
    "trash",
    "megaphone",
    "megaphone-off",
    "pencil",
    "refresh-cw",
    "check-check",
    "mail",
    "book",
    "search",
    "star",
    "star-filled",
  ];

  it("exports the published dictionary", () => {
    expect([...ICON_NAMES]).toEqual(PUBLISHED_ICONS);
  });

  it("renders the icon dictionary as non-interactive specimens", async () => {
    stubItem("icons");
    render(Detail, { props: { id: "icons" } });

    const catalog = await waitFor(() => {
      const el = document.querySelector(".catalog");
      if (!el) {
        throw new Error("catalog not rendered");
      }
      return el;
    });
    const tiles = [...catalog.querySelectorAll("li")];
    expect(tiles).toHaveLength(PUBLISHED_ICONS.length);
    expect(catalog.querySelectorAll("button, a, [tabindex]")).toHaveLength(0);
    for (const [position, name] of PUBLISHED_ICONS.entries()) {
      expect(screen.getByText(name)).toBeTruthy();
      const svg = tiles[position].querySelector("svg");
      expect(
        svg?.childElementCount,
        `specimen for "${name}" must draw shapes`,
      ).toBeGreaterThan(0);
    }
  });

  it("does not render the catalog for other items", async () => {
    stubItem("theme");
    render(Detail, { props: { id: "theme" } });

    await waitFor(() =>
      expect(screen.getByRole("heading", { name: "見本項目" })).toBeTruthy(),
    );
    expect(document.querySelector(".catalog")).toBeNull();
  });
});
