import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, symlink, rm } from "node:fs/promises";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { createStaticServer } from "./serve-static.ts";

test("static server confines files and HTML fallback to the intended surface", async (context) => {
  const temporary = await mkdtemp(join(tmpdir(), "forge-static-server-"));
  const root = join(temporary, "client");
  await mkdir(join(root, "assets"), { recursive: true });
  await mkdir(join(temporary, "client-other"));
  await writeFile(join(root, "_shell.html"), "<!doctype html><title>Synthetic shell</title>");
  await writeFile(join(root, "assets", "app.js"), "export const synthetic = true;");
  await writeFile(join(temporary, "private.txt"), "outside synthetic value");
  await writeFile(join(temporary, "client-other", "private.txt"), "outside prefix sibling");
  await symlink(join(temporary, "private.txt"), join(root, "assets", "escape.txt"));
  await symlink(join(temporary, "client-other"), join(root, "assets", "sibling"));
  const server = await createStaticServer(root);
  context.after(async () => {
    await new Promise<void>((resolve, reject) =>
      server.close((error) => (error ? reject(error) : resolve())),
    );
    await rm(temporary, { recursive: true, force: true });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  assert.ok(address && typeof address === "object");
  const port = address.port;

  // node:http preserves dot segments; fetch/URL would normalize away this attack.
  async function get(path: string, method = "GET", accept = "text/html") {
    return new Promise<{ status: number; body: string; type: string | undefined }>(
      (resolve, reject) => {
        const req = request(
          { host: "127.0.0.1", port, path, method, headers: { accept } },
          (res) => {
            let body = "";
            res.setEncoding("utf8");
            res.on("data", (chunk: string) => {
              body += chunk;
            });
            res.on("end", () =>
              resolve({ status: res.statusCode ?? 0, body, type: res.headers["content-type"] }),
            );
          },
        );
        req.on("error", reject);
        req.end();
      },
    );
  }

  for (const path of ["/", "/board", "/team", "/knowledge", "/tasks/TASK-142", "/board?tab=all"]) {
    const response = await get(path);
    assert.equal(response.status, 200, path);
    assert.match(response.body, /Synthetic shell/);
    assert.match(response.type ?? "", /^text\/html/);
  }
  const asset = await get("/assets/app.js");
  assert.equal(asset.status, 200);
  assert.match(asset.type ?? "", /javascript/);
  assert.match(asset.body, /synthetic/);
  assert.equal((await get("/board", "HEAD")).body, "");
  assert.equal((await get("/board", "POST")).status, 405);
  assert.equal((await get("/board", "GET", "application/json")).status, 404);

  for (const path of [
    "/assets/missing.js",
    "/assets/missing",
    "/favicon-missing.ico",
    "/unknown-route",
    "/api",
    "/api/tasks",
    "/v1",
    "/v1/tasks",
    "/_serverFn",
    "/_serverFn/example",
    "/assets/escape.txt",
    "/assets/sibling/private.txt",
    "/.env",
    "/src/routes/index.tsx",
  ]) {
    const response = await get(path);
    assert.equal(response.status, 404, path);
    assert.doesNotMatch(response.body, /Synthetic shell|outside/);
    assert.doesNotMatch(response.type ?? "", /html/);
  }
  for (const path of [
    "/../private.txt",
    "/assets/../../private.txt",
    "/%2e%2e/private.txt",
    "/assets/%2e%2e/%2e%2e/private.txt",
    "/%2e%2e%2fprivate.txt",
    "/assets%5c..%5c..%5cprivate.txt",
    "//private.txt",
    "/%00",
    "/%zz",
  ]) {
    const response = await get(path);
    assert.equal(response.status, 400, path);
    assert.doesNotMatch(response.body, /Synthetic shell|outside/);
  }
});
