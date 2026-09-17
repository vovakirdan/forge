import { test, expect } from "./fixtures";

test("owner terminal login reads the canonical Project under production CSP", async ({
  live,
  page,
}) => {
  const requests: string[] = [];
  page.on("request", (request) => requests.push(new URL(request.url()).origin));
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await page.getByLabel("Project ID", { exact: true }).fill(live.core.project_id);
  await page.getByRole("button", { name: "Load project", exact: true }).click();
  const project = page.getByRole("region", { name: "Project", exact: true });
  await expect(project).toContainText(live.core.project_id);
  await expect(project).toContainText(live.core.project_name);
  await expect(project.getByText(String(live.core.revision), { exact: true })).toBeVisible();
  await expect(project).toContainText(live.core.execution_gate);
  expect(await page.evaluate(() => "forgeInjected" in window)).toBe(false);
  expect(requests.every((origin) => origin === live.origin)).toBe(true);
  const shell = await fetch(live.origin);
  const csp = shell.headers.get("content-security-policy") ?? "";
  expect(csp).toContain("script-src 'self'");
  expect(csp).toContain("frame-ancestors 'none'");
  expect(csp).not.toContain("unsafe-inline");
  expect(csp).not.toContain("unsafe-eval");
  await live.allowConsoleErrors(
    /^(?:Executing inline script violates|Refused to execute inline script because it violates) the following Content Security Policy directive[\s\S]*script-src 'self'[\s\S]*$/,
    async () => {
      await page.addScriptTag({ content: "window.forgeCspBypass=true" }).catch(() => {});
      expect(await page.evaluate(() => "forgeCspBypass" in window)).toBe(false);
    },
  );
  await page.reload();
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
});

test("unauthorized, foreign-origin and non-allowlisted reads are refused", async ({ live }) => {
  expect((await fetch(`${live.origin}/api/health`)).status).toBe(401);
  const token = await live.bearer();
  for (const origin of ["null", "http://localhost:1234", "https://example.invalid"]) {
    const response = await fetch(`${live.origin}/api/projects/${live.core.project_id}`, {
      headers: { Authorization: `Bearer ${token}`, Origin: origin },
    });
    expect(response.status).toBe(403);
  }
  for (const path of ["/api/projects", "/v1/health", "/api/commands/start_project_execution"]) {
    const response = await fetch(`${live.origin}${path}`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    expect(response.status).toBe(404);
    expect(response.headers.get("content-type")).not.toContain("text/html");
  }
});

test("unknown Project has no mock fallback and network failure remains explicit", async ({
  live,
  page,
}) => {
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 404 \(Not Found\)$/,
    async () => {
      await page
        .getByLabel("Project ID", { exact: true })
        .fill("01900000-0000-7000-8000-000000000001");
      await page.getByRole("button", { name: "Load project", exact: true }).click();
      await expect(page.getByRole("alert")).toContainText(/not found/i);
    },
  );
  await expect(page.getByRole("region", { name: "Project", exact: true })).toHaveCount(0);
  await page.context().setOffline(true);
  await live.allowConsoleErrors(
    /^Failed to load resource: net::ERR_INTERNET_DISCONNECTED$/,
    async () => {
      await page.getByLabel("Project ID", { exact: true }).fill(live.core.project_id);
      await page.getByRole("button", { name: "Load project", exact: true }).click();
      await expect(page.getByRole("alert")).toContainText(/unavailable|network|connect/i);
    },
  );
  await expect(page.getByRole("button", { name: "Log out", exact: true })).toBeVisible();
  await page.context().setOffline(false);
  await page.getByRole("button", { name: "Retry project", exact: true }).click();
  await expect(page.getByRole("region", { name: "Project", exact: true })).toContainText(
    live.core.project_id,
  );
});

test("logout revokes a session copied into another tab", async ({ live, page }) => {
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  const opened = page.waitForEvent("popup");
  await page.evaluate(() => {
    window.open(window.location.href, "_blank");
  });
  const copied = await opened;
  await expect(copied.getByText("Connected to Forge", { exact: true })).toBeVisible();
  const revoked = page.waitForResponse(
    (response) =>
      response.url() === `${live.origin}/api/auth/logout` && response.request().method() === "POST",
  );
  await page.getByRole("button", { name: "Log out", exact: true }).click();
  expect((await revoked).status()).toBe(204);
  await expect(page.getByRole("heading", { name: "Connect to Forge" })).toBeVisible();
  await live.allowConsoleErrors(
    /^Failed to load resource: the server responded with a status of 401 \(Unauthorized\)$/,
    async () => {
      await copied.reload();
      await expect(copied.getByRole("heading", { name: "Connect to Forge" })).toBeVisible();
    },
  );
  await expect(copied.getByRole("region", { name: "Project", exact: true })).toHaveCount(0);
});

test("unavailable sessionStorage fails closed", async ({ live, page }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, "sessionStorage", {
      get: () => {
        throw new Error("synthetic storage denial");
      },
    });
  });
  await page.goto(live.origin);
  await expect(page.getByRole("alert")).toContainText(/storage/i);
  await expect(page.getByRole("heading", { name: "Live connection" })).toHaveCount(0);
});

test("offline logout clears local session without claiming server revocation", async ({
  live,
  page,
}) => {
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  await page.context().setOffline(true);
  await live.allowConsoleErrors(
    /^Failed to load resource: net::ERR_INTERNET_DISCONNECTED$/,
    async () => {
      await page.getByRole("button", { name: "Log out", exact: true }).click();
      await expect(page.getByRole("heading", { name: "Connect to Forge" })).toBeVisible();
      await expect(page.getByRole("alert")).toContainText(/not confirmed|could not|unconfirmed/i);
    },
  );
  await page.context().setOffline(false);
  await page.reload();
  await expect(page.getByRole("heading", { name: "Connect to Forge" })).toBeVisible();
});

test("switching Project cancels a delayed real response without restoring the previous card", async ({
  live,
  page,
}) => {
  await live.login(page);
  await expect(page.getByText("Connected to Forge", { exact: true })).toBeVisible();
  const previousUrl = `${live.origin}/api/projects/${live.core.project_id}`;
  let release!: () => void;
  let captured!: () => void;
  let finished!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  const responseCaptured = new Promise<void>((resolve) => {
    captured = resolve;
  });
  const responseFinished = new Promise<void>((resolve) => {
    finished = resolve;
  });
  let cancelled = false;
  let failedDelivery = false;
  page.on("requestfailed", (request) => {
    if (request.url() === previousUrl && request.failure()?.errorText === "net::ERR_ABORTED")
      cancelled = true;
  });
  await page.route(previousUrl, async (route) => {
    try {
      // Delay a response from the actual gateway/Core; never fabricate Project data.
      const response = await route.fetch({ timeout: 10_000 });
      if (response.status() !== 200) {
        failedDelivery = true;
        captured();
        return;
      }
      captured();
      await held;
      try {
        await route.fulfill({ response });
      } catch {
        if (!cancelled) failedDelivery = true;
      }
    } catch {
      // Route errors can include request headers; expose only a safe test flag.
      failedDelivery = true;
      captured();
    } finally {
      finished();
    }
  });
  try {
    await page.getByLabel("Project ID", { exact: true }).fill(live.core.project_id);
    await page.getByRole("button", { name: "Load project", exact: true }).click();
    await responseCaptured;
    expect(failedDelivery, "The real gateway response was captured").toBe(false);
    await live.allowConsoleErrors(
      /^Failed to load resource: the server responded with a status of 404 \(Not Found\)$/,
      async () => {
        await page
          .getByLabel("Project ID", { exact: true })
          .fill("01900000-0000-7000-8000-000000000001");
        await page.getByRole("button", { name: "Load project", exact: true }).click();
        await expect(page.getByRole("alert")).toContainText(/not found/i);
      },
    );
    await expect.poll(() => cancelled).toBe(true);
    release();
    await responseFinished;
    expect(failedDelivery).toBe(false);
    await expect(page.getByRole("region", { name: "Project", exact: true })).toHaveCount(0);
    await expect(page.getByRole("alert")).toContainText(/not found/i);
  } finally {
    release();
    await page.unrouteAll({ behavior: "wait" });
  }
});
