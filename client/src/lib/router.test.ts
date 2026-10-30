import { waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import { initRouter, navigate, router } from "./router.svelte";
import { matchRoute } from "./routes";

describe("matchRoute", () => {
  it("maps / to the home route", () => {
    expect(matchRoute("/")).toEqual({ index: 0, params: {} });
  });

  it("maps /items/:id to the detail route with its id", () => {
    expect(matchRoute("/items/sumi")).toEqual({
      index: 1,
      params: { id: "sumi" },
    });
  });

  it("normalizes unknown paths to home", () => {
    expect(matchRoute("/no/such/page")).toEqual({ index: 0, params: {} });
  });
});

describe("router", () => {
  afterEach(() => {
    window.history.replaceState(null, "", "/");
  });

  it("navigate() pushes history and updates the route state", () => {
    const teardown = initRouter();

    navigate("/items/kinari");

    expect(window.location.pathname).toBe("/items/kinari");
    expect(router.index).toBe(1);
    expect(router.params.id).toBe("kinari");
    teardown();
  });

  it("the browser back button returns to the previous route", async () => {
    const teardown = initRouter();
    window.history.replaceState(null, "", "/");
    navigate("/items/kinari");

    window.history.back();

    await waitFor(() => expect(router.index).toBe(0));
    expect(window.location.pathname).toBe("/");
    teardown();
  });

  it("intercepts clicks on internal links", () => {
    const teardown = initRouter();
    const anchor = document.createElement("a");
    anchor.href = "/items/sumi";
    anchor.textContent = "card";
    document.body.appendChild(anchor);

    anchor.click();

    expect(window.location.pathname).toBe("/items/sumi");
    expect(router.index).toBe(1);
    anchor.remove();
    teardown();
  });
});
