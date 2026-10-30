import { waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import { initRouter, navigate, router } from "./router.svelte";
import { matchRoute } from "./routes";

describe("matchRoute", () => {
  it("maps / to the home route", () => {
    expect(matchRoute("/")).toEqual({ index: 0, params: {} });
  });

  it("maps /view/* to the viewer with the decoded base camp path", () => {
    expect(matchRoute("/view/renders/%E3%83%A2%E3%82%AB%20a%2Fb.png")).toEqual({
      index: 1,
      params: { path: "renders/モカ a/b.png" },
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

    navigate("/view/renders/kinari.png");

    expect(window.location.pathname).toBe("/view/renders/kinari.png");
    expect(router.index).toBe(1);
    expect(router.params.path).toBe("renders/kinari.png");
    teardown();
  });

  it("the browser back button returns to the previous route", async () => {
    const teardown = initRouter();
    window.history.replaceState(null, "", "/");
    navigate("/view/renders/kinari.png");

    window.history.back();

    await waitFor(() => expect(router.index).toBe(0));
    expect(window.location.pathname).toBe("/");
    teardown();
  });

  it("intercepts clicks on internal links", () => {
    const teardown = initRouter();
    const anchor = document.createElement("a");
    anchor.href = "/view/renders/sumi.png";
    anchor.textContent = "card";
    document.body.appendChild(anchor);

    anchor.click();

    expect(window.location.pathname).toBe("/view/renders/sumi.png");
    expect(router.index).toBe(1);
    anchor.remove();
    teardown();
  });
});
