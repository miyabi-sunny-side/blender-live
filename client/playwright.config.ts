import { resolve } from "node:path";

import { defineConfig } from "@playwright/test";

// The real server with the stub Blender from tests/, on a throwaway camp.
const PORT = 5188;
const ROOT = resolve(import.meta.dirname, "..");
export const CAMP = resolve(ROOT, "target/e2e-camp");

export default defineConfig({
  testDir: "./e2e",
  workers: 1,
  use: { baseURL: `http://127.0.0.1:${PORT}`, browserName: "chromium" },
  webServer: {
    command: `rm -rf ${CAMP} && cargo run -q --manifest-path ../Cargo.toml -- ${CAMP}`,
    env: {
      PORT: String(PORT),
      LOG_LEVEL: "warn",
      BLENDER_BIN: resolve(ROOT, "tests/fake_blender.py"),
    },
    url: `http://127.0.0.1:${PORT}/healthz`,
    timeout: 300_000,
  },
});
