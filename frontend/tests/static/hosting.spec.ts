import { request as rawRequest } from "node:http";
import { expect, test } from "../browser/fixtures";

for (const route of [
  { path: "/board", heading: "Engineering Board" },
  { path: "/team", heading: "Team" },
  { path: "/knowledge", heading: "Knowledge" },
  { path: "/tasks/TASK-142", heading: /^TASK-142 — Track partial moves/ },
]) {
  test(`static direct entry and reload: ${route.path}`, async ({ page }) => {
    const response = await page.goto(route.path);
    expect(response?.status()).toBe(200);
    await expect(page.getByRole("heading", { name: route.heading, exact: true })).toBeVisible();
    await page.reload();
    await expect(page.getByRole("heading", { name: route.heading, exact: true })).toBeVisible();
    await expect(page).toHaveURL(route.path);
  });
}

test("static hosting never rewrites missing assets or server namespaces to HTML", async ({
  request,
}) => {
  for (const path of [
    "/assets/not-present.js",
    "/assets/not-present",
    "/not-present.css",
    "/api",
    "/api/tasks",
    "/v1",
    "/v1/tasks",
    "/_serverFn",
    "/_serverFn/not-present",
  ]) {
    const response = await request.get(path, { headers: { Accept: "text/html" } });
    expect(response.status(), path).toBe(404);
    expect(response.headers()["content-type"]).not.toContain("text/html");
    expect(await response.text()).not.toContain("<!DOCTYPE html>");
  }
});

test("static hosting rejects raw root escapes before URL normalization", async ({ baseURL }) => {
  if (!baseURL) throw new Error("Static baseURL must be configured");
  const origin = new URL(baseURL);
  for (const path of [
    "/../server/server.js",
    "/%2e%2e/server/server.js",
    "/assets/%2e%2e/%2e%2e/package.json",
  ]) {
    const response = await new Promise<{ status: number; body: string }>((resolve, reject) => {
      const req = rawRequest({ hostname: origin.hostname, port: origin.port, path }, (res) => {
        let body = "";
        res.setEncoding("utf8");
        res.on("data", (chunk: string) => {
          body += chunk;
        });
        res.on("end", () => resolve({ status: res.statusCode ?? 0, body }));
      });
      req.on("error", reject);
      req.end();
    });
    expect(response.status, path).toBe(400);
    expect(response.body).toBe("Bad request\n");
  }
});
