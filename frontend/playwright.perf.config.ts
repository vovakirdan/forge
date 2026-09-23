import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/perf",
  testMatch: "*.spec.ts",
  workers: 1,
  retries: 0,
  forbidOnly: true,
  timeout: 180_000,
  globalTimeout: 900_000,
  reporter: "line",
  outputDir: "test-results/perf",
  use: {
    browserName: "chromium",
    viewport: { width: 1280, height: 800 },
    trace: "off",
    screenshot: "off",
    video: "off",
    serviceWorkers: "block",
  },
});
