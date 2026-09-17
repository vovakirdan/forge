import assert from "node:assert/strict";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { resolveConfig } from "vite";

test("static config disables automatic build-environment injection and binds prerender to loopback", async (context) => {
  const key = "VITE_FORGE_STATIC_SENTINEL";
  const previous = process.env[key];
  process.env[key] = "synthetic-client-sentinel-not-secret";
  context.after(() => {
    if (previous === undefined) delete process.env[key];
    else process.env[key] = previous;
  });
  const config = await resolveConfig(
    { configFile: fileURLToPath(new URL("../vite.static.config.ts", import.meta.url)) },
    "build",
    "production",
  );
  assert.equal(config.envDir, false);
  assert.deepEqual(config.envPrefix, []);
  assert.deepEqual(Object.keys(config.env).sort(), ["BASE_URL", "DEV", "MODE", "PROD"]);
  assert.equal(config.define?.[`import.meta.env.${key}`], undefined);
  assert.equal(config.preview.host, "127.0.0.1");
});
