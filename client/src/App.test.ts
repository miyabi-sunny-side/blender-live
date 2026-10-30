import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "./App.svelte";

class SilentEventSource {
  onmessage = null;
  onopen = null;
  close() {}
}

describe("App", () => {
  afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    window.history.replaceState(null, "", "/");
  });

  it("keeps the invariant header and restores a deep viewer URL", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>().mockResolvedValue(new Response("[]")),
    );
    vi.stubGlobal("EventSource", SilentEventSource);
    window.history.replaceState(null, "", "/view/renders/cube.png");

    render(App);

    const header = screen.getByRole("banner");
    const title = header.querySelector('a[href="/"]');
    expect(title?.textContent).toContain("blender-live");
    expect(screen.getByRole("button", { name: "メニュー" })).toBeTruthy();
    expect(header.querySelectorAll("a, button")).toHaveLength(2);
    expect(screen.getByRole("heading", { name: "cube.png" })).toBeTruthy();

    await fireEvent.click(title as HTMLElement);
    expect(window.location.pathname).toBe("/");
    expect(screen.getByRole("heading", { name: "書き出し" })).toBeTruthy();
  });
});
