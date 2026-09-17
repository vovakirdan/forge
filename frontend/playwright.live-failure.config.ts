import { defineConfig } from "@playwright/test";
import live from "./playwright.live.config";

export default defineConfig({
  ...live,
  testMatch: "failure.probe.ts",
  outputDir: "test-results/live-failure",
  reporter: [["line"], ["./scripts/live-failure-reporter.ts"]],
});
