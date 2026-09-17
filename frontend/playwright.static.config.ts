import { defineConfig } from "@playwright/test";
import browserConfig from "./playwright.config";

export default defineConfig({
  ...browserConfig,
  testDir: "./tests",
  testMatch: ["browser/demo.spec.ts", "static/*.spec.ts"],
  outputDir: "test-results/static",
  reporter: [["line"], ["html", { open: "never", outputFolder: "playwright-report/static" }]],
  use: { ...browserConfig.use, baseURL: "http://127.0.0.1:4174" },
  webServer: {
    command: "bun run test:static:serve",
    url: "http://127.0.0.1:4174/_shell.html",
    reuseExistingServer: false,
    timeout: 120_000,
    gracefulShutdown: { signal: "SIGTERM", timeout: 5_000 },
  },
});
