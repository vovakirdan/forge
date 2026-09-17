import { test as base, expect, type ConsoleMessage } from "@playwright/test";

export const test = base.extend<{ browserErrors: void }>({
  browserErrors: [
    async ({ page }, use, testInfo) => {
      const errors: string[] = [];
      const onError = (error: Error) => errors.push(`pageerror: ${error.message}`);
      const onConsole = (message: ConsoleMessage) => {
        if (message.type() === "error") errors.push(`console.error: ${message.text()}`);
      };
      page.on("pageerror", onError);
      page.on("console", onConsole);
      await use();
      page.off("pageerror", onError);
      page.off("console", onConsole);
      if (errors.length > 0) {
        await testInfo.attach("browser-errors", {
          body: JSON.stringify(errors, null, 2),
          contentType: "application/json",
        });
      }
      expect(errors, "No uncaught browser exceptions or console errors").toEqual([]);
    },
    { auto: true },
  ],
});

export { expect };
