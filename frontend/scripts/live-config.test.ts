import assert from "node:assert/strict";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { resolveConfig } from "vite";

test("live build is a separate static entry without env exports, public files, SSR or demo plugins", async (context) => {
  const key = "VITE_FORGE_LIVE_SENTINEL";
  const previous = process.env[key];
  process.env[key] = "synthetic-client-sentinel";
  context.after(() => {
    if (previous === undefined) delete process.env[key];
    else process.env[key] = previous;
  });
  const config = await resolveConfig(
    { configFile: fileURLToPath(new URL("../vite.live.config.ts", import.meta.url)) },
    "build",
    "production",
  );
  assert.equal(config.envDir, false);
  assert.deepEqual(config.envPrefix, []);
  assert.deepEqual(Object.keys(config.env).sort(), ["BASE_URL", "DEV", "MODE", "PROD"]);
  assert.equal(config.define?.[`import.meta.env.${key}`], undefined);
  assert.equal(config.publicDir, "");
  assert.equal(config.root, fileURLToPath(new URL("../src/live", import.meta.url)));
  assert.equal(config.build.outDir, fileURLToPath(new URL("../dist-live", import.meta.url)));
  assert.equal(config.build.ssr, false);
  assert.equal(
    config.plugins.some((plugin) => /lovable|tanstack|nitro/i.test(plugin.name)),
    false,
  );
});
