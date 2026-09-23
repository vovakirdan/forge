import { runBounded } from "./live-process.ts";
import { chmod, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const frontend = fileURLToPath(new URL("..", import.meta.url));
const directory = await mkdtemp(join(tmpdir(), "forge-ui-markers-"));
await chmod(directory, 0o700);
const socketPath = join(directory, "scan.sock");
const secrets = new Set<string>();
const interrupted = new AbortController();
const interrupt = () => interrupted.abort();
process.once("SIGINT", interrupt);
process.once("SIGTERM", interrupt);
let registryFailed = false;
const registry = createServer((socket) => {
  let input = "";
  socket.setTimeout(2000, () => socket.destroy());
  socket.on("error", () => {
    registryFailed = true;
  });
  socket.on("data", (chunk: Buffer) => {
    input += chunk.toString();
    if (input.length > 4096) {
      registryFailed = true;
      socket.destroy();
    }
  });
  socket.on("end", () => {
    try {
      const values: unknown = JSON.parse(input);
      if (
        !Array.isArray(values) ||
        values.length > 32 ||
        !values.every((value) => typeof value === "string" && /^[a-f0-9]{64}$/.test(value))
      ) {
        registryFailed = true;
      } else for (const value of values) secrets.add(value as string);
    } catch {
      registryFailed = true;
    }
  });
});
registry.maxConnections = 8;
await new Promise<void>((resolve, reject) => {
  registry.once("error", reject);
  registry.listen(socketPath, resolve);
});
await chmod(socketPath, 0o600);

function sanitize(text: string) {
  let safe = text;
  for (const secret of secrets) safe = safe.replaceAll(secret, "[redacted]");
  return safe;
}

async function scan(path: string): Promise<number> {
  let leaks = 0;
  for (const entry of await readdir(path, { withFileTypes: true }).catch(() => [])) {
    const child = join(path, entry.name);
    if (entry.isDirectory()) leaks += await scan(child);
    else if (entry.isFile()) {
      const original = await readFile(child);
      const text = original.toString();
      const safe = sanitize(text);
      if (safe !== text) {
        // Retain safe diagnostics while still failing the confidentiality gate.
        await writeFile(child, safe);
        leaks += 1;
      }
    }
  }
  return leaks;
}

async function suite(config: string, expectedExit: number) {
  const priorSecrets = secrets.size;
  if (interrupted.signal.aborted) throw new Error("Live acceptance interrupted");
  const result = await runBounded(
    process.execPath,
    ["node_modules/@playwright/test/cli.js", "test", "--config", config],
    {
      cwd: frontend,
      env: {
        PATH: process.env["PATH"] ?? "/usr/bin:/bin",
        ...(process.env["HOME"] ? { HOME: process.env["HOME"] } : {}),
        ...(process.env["XDG_RUNTIME_DIR"]
          ? { XDG_RUNTIME_DIR: process.env["XDG_RUNTIME_DIR"] }
          : {}),
        LANG: "C.UTF-8",
        FORGE_UI_TEST_REGISTRY: socketPath,
      },
      signal: interrupted.signal,
      timeoutMs: 620_000,
    },
  );
  let { output } = result;
  const { status, excessive, stopped } = result;
  const safe = sanitize(output);
  const leakedOutput = safe !== output;
  const proof = /^FORGE_FAILURE_PROBE (\{[^\n]+\})$/m.exec(safe)?.[1];
  let failureCounts: unknown;
  try {
    failureCounts = proof ? JSON.parse(proof) : null;
  } catch {
    failureCounts = null;
  }
  const exercisedFailure =
    expectedExit !== 1 ||
    (failureCounts !== null &&
      typeof failureCounts === "object" &&
      "tests" in failureCounts &&
      failureCounts.tests === 1 &&
      "expected" in failureCounts &&
      failureCounts.expected === 1 &&
      "unexpected" in failureCounts &&
      failureCounts.unexpected === 0 &&
      "status" in failureCounts &&
      failureCounts.status === "failed" &&
      secrets.size >= priorSecrets + 3);
  process.stdout.write(safe);
  output = "";
  const leakedFiles = await scan(join(frontend, "test-results"));
  const leakedAssets = await scan(join(frontend, "dist-live"));
  if (
    status !== expectedExit ||
    stopped ||
    interrupted.signal.aborted ||
    !exercisedFailure ||
    excessive ||
    registryFailed ||
    leakedOutput ||
    leakedFiles ||
    leakedAssets ||
    secrets.size === 0
  ) {
    throw new Error("Live acceptance failed: exit status, output bounds or secret scan");
  }
  console.log(
    `Secret scan PASS: ${secrets.size} issued values; ${expectedExit === 1 ? "intentional failure" : "live suite"}, no leaks.`,
  );
}

try {
  await suite("playwright.live.config.ts", 0);
  await suite("playwright.live-failure.config.ts", 1);
} catch (error) {
  console.error(error instanceof Error ? sanitize(error.message) : "Live acceptance failed");
  process.exitCode = 1;
} finally {
  await new Promise<void>((resolve) => registry.close(() => resolve()));
  await rm(directory, { recursive: true, force: true });
}
