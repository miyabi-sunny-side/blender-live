import { cleanup, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import View from "./View.svelte";

afterEach(cleanup);

describe("View", () => {
  it("shows an image with its name and a download link", () => {
    render(View, { path: "renders/モカ 正面.png" });
    expect(screen.getByRole("heading", { name: "モカ 正面.png" })).toBeTruthy();
    const img = screen.getByRole("img", { name: "モカ 正面.png" });
    expect(img.getAttribute("src")).toBe(
      "/files/renders/%E3%83%A2%E3%82%AB%20%E6%AD%A3%E9%9D%A2.png",
    );
    expect(
      screen.getByRole("link", { name: "ダウンロード" }).getAttribute("href"),
    ).toBe(
      "/files/renders/%E3%83%A2%E3%82%AB%20%E6%AD%A3%E9%9D%A2.png?download=1",
    );
  });

  it("plays videos with controls", () => {
    render(View, { path: "renders/turntable.mp4" });
    const video = document.querySelector("video");
    expect(video?.getAttribute("src")).toBe("/files/renders/turntable.mp4");
    expect(video?.hasAttribute("controls")).toBe(true);
  });
});
