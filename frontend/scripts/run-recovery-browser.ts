import { chmod, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { runBounded } from "./live-process.ts";

const frontend = fileURLToPath(new URL("..", import.meta.url));
const directory = await mkdtemp(join(tmpdir(), "forge-recovery-browser-"));
await chmod(directory, 0o700);
const socket = join(directory, "markers.sock");
const secrets = new Set<string>();
let registryFailed = false;
const registry = createServer((connection) => {
  let input = "";
  connection.setTimeout(2000, () => connection.destroy());
  connection.on("data", (chunk: Buffer) => {
    input += chunk.toString();
    if (input.length > 4096) {
      registryFailed = true;
      connection.destroy();
    }
  });
  connection.on("end", () => {
    try {
      const values: unknown = JSON.parse(input);
      if (
        !Array.isArray(values) ||
        values.length > 32 ||
        !values.every((value) => typeof value === "string" && /^[a-f0-9]{64}$/.test(value))
      )
        registryFailed = true;
      else for (const value of values) secrets.add(value as string);
    } catch {
      registryFailed = true;
    }
  });
});
await new Promise<void>((resolve, reject) => {
  registry.once("error", reject);
  registry.listen(socket, resolve);
});
await chmod(socket, 0o600);
const interrupted = new AbortController();
process.once("SIGINT", () => interrupted.abort());
process.once("SIGTERM", () => interrupted.abort());
async function scan(path: string): Promise<boolean> {
  let leaked = false;
  for (const entry of await readdir(path, { withFileTypes: true }).catch(() => [])) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) leaked = (await scan(child)) || leaked;
    else if (entry.isFile()) {
      const original = await readFile(child);
      let safe = original.toString();
      for (const secret of secrets) safe = safe.replaceAll(secret, "[redacted]");
      if (safe !== original.toString()) {
        leaked = true;
        await writeFile(child, safe);
      }
    }
  }
  return leaked;
}
try {
  const result = await runBounded(
    process.execPath,
    [
      "node_modules/@playwright/test/cli.js",
      "test",
      "--config",
      "playwright.live.config.ts",
      "tests/live/recovery-assessment.spec.ts",
    ],
    {
      cwd: frontend,
      env: {
        PATH: process.env["PATH"] ?? "/usr/bin:/bin",
        LANG: "C.UTF-8",
        FORGE_UI_TEST_REGISTRY: socket,
      },
      signal: interrupted.signal,
      timeoutMs: 180_000,
    },
  );
  let output = result.output;
  let leaked = false;
  for (const secret of secrets)
    if (output.includes(secret)) {
      output = output.replaceAll(secret, "[redacted]");
      leaked = true;
    }
  process.stdout.write(output);
  const artifactLeak =
    (await scan(join(frontend, "test-results/live"))) || (await scan(join(frontend, "dist-live")));
  if (
    result.status !== 0 ||
    result.stopped ||
    result.excessive ||
    registryFailed ||
    leaked ||
    artifactLeak ||
    secrets.size === 0
  )
    throw Error("Targeted recovery browser proof failed");
} catch (error) {
  console.error(error instanceof Error ? error.message : "Targeted recovery browser proof failed");
  process.exitCode = 1;
} finally {
  await new Promise<void>((resolve) => registry.close(() => resolve()));
  await rm(directory, { recursive: true, force: true });
}
