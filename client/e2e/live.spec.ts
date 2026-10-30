import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test, type APIRequestContext } from "@playwright/test";

import { CAMP } from "../playwright.config";

async function mcp(
  request: APIRequestContext,
  name: string,
  args: Record<string, unknown>,
): Promise<string> {
  const headers = {
    "content-type": "application/json",
    accept: "application/json, text/event-stream",
  };
  const init = await request.post("/mcp", {
    headers,
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: {
        protocolVersion: "2025-06-18",
        capabilities: {},
        clientInfo: { name: "e2e", version: "0" },
      },
    },
  });
  const session = {
    ...headers,
    "mcp-session-id": init.headers()["mcp-session-id"],
  };
  await request.post("/mcp", {
    headers: session,
    data: { jsonrpc: "2.0", method: "notifications/initialized" },
  });
  const call = await request.post("/mcp", {
    headers: session,
    data: {
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: { name, arguments: args },
    },
  });
  return call.text();
}

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(page.getByText("待機中")).toBeVisible();
});

test("a new render appears without reloading the page", async ({ page }) => {
  await page.evaluate(() => {
    (window as unknown as { marker: number }).marker = 1;
  });
  writeFileSync(join(CAMP, "renders", `fresh-${Date.now()}.png`), "");
  await mcp(page.request, "render_still", { name: "from-mcp" });
  await expect(
    page.locator(".render .name", { hasText: "from-mcp.png" }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => (window as unknown as { marker?: number }).marker,
    ),
  ).toBe(1);
  const first = page.locator(".render .name").first();
  await expect(first).toHaveText(/from-mcp\.png|fresh-/);
});

test("a job shows progress, can be cancelled, and a finished video plays", async ({
  page,
}) => {
  await mcp(page.request, "render_animation", {
    name: "long",
    frame_start: 1,
    frame_end: 400,
    fps: 24,
  });
  const bar = page.getByRole("progressbar", { name: "long の進み具合" });
  await expect(bar).toBeVisible();
  await expect
    .poll(async () => Number(await bar.getAttribute("aria-valuenow")))
    .toBeGreaterThan(0);
  await page.getByRole("button", { name: "long を取り消す" }).click();
  await expect(
    page.locator(".job", { hasText: "long" }).getByText("取り消し"),
  ).toBeVisible();

  await mcp(page.request, "render_animation", {
    name: "short",
    frame_start: 1,
    frame_end: 12,
    fps: 12,
  });
  const job = page.locator(".job", { hasText: "short" });
  await expect(job.getByText("完了")).toBeVisible({ timeout: 20_000 });
  const video = page.locator('.render[data-kind="video"] video').first();
  await expect(video).toHaveAttribute("src", "/files/renders/short.mp4");
  const played = await video.evaluate(async (v: HTMLVideoElement) => {
    v.muted = true;
    await v.play();
    await new Promise((r) => setTimeout(r, 300));
    return { time: v.currentTime, duration: v.duration };
  });
  expect(played.time).toBeGreaterThan(0);
  expect(played.duration).toBeCloseTo(1, 1);
});

test("a crash is reported with the lost scene", async ({ page }) => {
  await mcp(page.request, "python", { code: "x = 1" });
  await mcp(page.request, "python", { code: "import os\nos._exit(1)" });
  await expect(page.getByRole("alert")).toContainText(
    "未保存の scene は失われました",
  );
  await expect(page.getByText("待機中")).toBeVisible();
});

test("files outside the base camp are refused", async ({ request }) => {
  for (const path of [
    "/files/..%2F..%2FCargo.toml",
    "/files/%2Fetc%2Fhostname",
  ]) {
    expect((await request.get(path)).status(), path).toBe(403);
  }
  const text = await mcp(request, "save_blend", { path: "../outside.blend" });
  expect(text).toContain("refused");
});

for (const [width, height] of [
  [1440, 900],
  [375, 812],
  [320, 640],
]) {
  for (const scheme of ["dark", "light"] as const) {
    test(`layout at ${width}px in ${scheme}`, async ({ page }, info) => {
      writeFileSync(join(CAMP, "renders", `${"long-name-".repeat(8)}.png`), "");
      await page.emulateMedia({ colorScheme: scheme });
      await page.setViewportSize({ width, height });
      await page.goto("/");
      await expect(page.getByText("待機中")).toBeVisible();
      await expect(page.locator(".render").first()).toBeVisible();
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - window.innerWidth,
      );
      expect(overflow).toBe(0);
      const header = page.getByRole("banner");
      await expect(header.locator("a, button")).toHaveCount(2);
      await info.attach(`home-${width}-${scheme}`, {
        body: await page.screenshot({ fullPage: true }),
        contentType: "image/png",
      });
    });
  }
}
