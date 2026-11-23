import { resolve } from "node:path";

import { defineConfig } from "@playwright/test";

// The real server with the stub Blender from tests/, on a throwaway camp.
// A second one hands the viewing of renders to an outside page.
const PORT = 5188;
const ROOT = resolve(import.meta.dirname, "..");
export const CAMP = resolve(ROOT, "target/e2e-camp");
export const OUTSIDE = {
  origin: "http://127.0.0.1:5189",
  camp: resolve(ROOT, "target/e2e-camp-outside"),
  viewer: "http://depot.test:5015/view/blender",
};

function server(port: number, camp: string, env: Record<string, string>) {
  return {
    command: `rm -rf ${camp} && cargo run -q --manifest-path ../Cargo.toml -- ${camp}`,
    env: {
      PORT: String(port),
      LOG_LEVEL: "warn",
      BLENDER_BIN: resolve(ROOT, "tests/fake_blender.py"),
      ...env,
    },
    url: `http://127.0.0.1:${port}/healthz`,
    timeout: 300_000,
  };
}

export default defineConfig({
  testDir: "./e2e",
  workers: 1,
  use: { baseURL: `http://127.0.0.1:${PORT}`, browserName: "chromium" },
  webServer: [
    server(PORT, CAMP, {}),
    server(5189, OUTSIDE.camp, { VIEWER_URL: OUTSIDE.viewer }),
  ],
});
