import { defineConfig } from "@playwright/test";

/** Drives the real UI against a real server (D13). */
export default defineConfig({
  testDir: "./e2e",
  timeout: 30_000,
  use: {
    baseURL: process.env.CAIRN_WEB_URL ?? "http://127.0.0.1:3100",
    trace: "retain-on-failure",
  },
  reporter: [["list"]],
});
