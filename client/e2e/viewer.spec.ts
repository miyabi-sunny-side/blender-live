import { writeFileSync } from "node:fs";
import { join } from "node:path";

import { expect, test } from "@playwright/test";

import { OUTSIDE } from "../playwright.config";

test("with an outside viewer, renders are links to it and tools return its address", async ({
  page,
}) => {
  const name = `still-${Date.now()}.png`;
  writeFileSync(join(OUTSIDE.camp, "renders", name), "png");

  await page.goto(`${OUTSIDE.origin}/`);
  await expect(page.getByText("待機中")).toBeVisible();
  const link = page.getByRole("link", { name });
  await expect(link).toHaveAttribute("href", `${OUTSIDE.viewer}/${name}`);
  await expect(page.locator(".gallery, video")).toHaveCount(0);
  // The other base camp files stay on this page.
  await expect(page.getByRole("tab", { name: "scenes" })).toBeVisible();

  const headers = {
    "content-type": "application/json",
    accept: "application/json, text/event-stream",
  };
  const init = await page.request.post(`${OUTSIDE.origin}/mcp`, {
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
  await page.request.post(`${OUTSIDE.origin}/mcp`, {
    headers: session,
    data: { jsonrpc: "2.0", method: "notifications/initialized" },
  });
  const call = await page.request.post(`${OUTSIDE.origin}/mcp`, {
    headers: session,
    data: {
      jsonrpc: "2.0",
      id: 2,
      method: "tools/call",
      params: { name: "render_still", arguments: { name: "from-mcp" } },
    },
  });
  const text = await call.text();
  expect(text).toContain(`${OUTSIDE.viewer}/from-mcp.png`);
  expect(text).toContain("/files/renders/from-mcp.png");
});
