import { defineConfig, devices } from "@playwright/test";

/**
 * Drives the real UI against a real server (D13).
 *
 * Point it at a **release** build of `cairn-server`. Argon2 is deliberately
 * expensive, and unoptimized it costs ~0.7s per sign-in against ~0.03s
 * released — which under parallel workers pushes sign-in past the assertion
 * timeout and looks like a flaky UI rather than a slow hash.
 */
export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  // Every test signs in for real, and a real sign-in is an argon2 verify. The
  // 5s default is a fine budget for a mocked UI and a tight one for this, so a
  // busy machine reads as a flaky interface. No assertion is relaxed by this —
  // only how long each is given to become true.
  expect: { timeout: 10_000 },
  use: {
    baseURL: process.env.CAIRN_WEB_URL ?? "http://127.0.0.1:3100",
    trace: "retain-on-failure",
  },
  reporter: [["list"]],
  // The shell changes shape at the mobile breakpoint — the sidebar becomes a
  // sheet — so the same flows are proved on both rather than assumed.
  projects: [
    {
      name: "desktop-chromium",
      use: { ...devices["Desktop Chrome"], viewport: { width: 1280, height: 800 } },
    },
    {
      name: "mobile-chromium",
      use: { ...devices["Pixel 5"] },
    },
  ],
});
