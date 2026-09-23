import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/live",
  testMatch: "*.spec.ts",
  fullyParallel: false,
  workers: 1,
  retries: 0,
  forbidOnly: true,
  timeout: 45_000,
  globalTimeout: 600_000,
  reporter: "line",
  outputDir: "test-results/live",
  use: {
    browserName: "chromium",
    viewport: { width: 1280, height: 800 },
    trace: "off",
    screenshot: "off",
    video: "off",
    serviceWorkers: "block",
  },
});
