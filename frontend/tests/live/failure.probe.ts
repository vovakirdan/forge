// Intentionally run in a separate process: exit 1 is required, not a passing test.
import { test, expect } from "./fixtures";

test("intentional authenticated failure leaves no issued secret in artifacts", async ({
  live,
  page,
}) => {
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  const token = await live.bearer();
  // Exercise error redaction with actual issued material, not a fixed sentinel.
  throw new Error(`Intentional probe ${token} ${[...live.secrets][0]}`);
});
