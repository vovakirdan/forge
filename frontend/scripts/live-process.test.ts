import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { setTimeout as delay } from "node:timers/promises";
import { runBounded } from "./live-process.ts";

test("interruption escalates for a process that ignores TERM", async () => {
  const result = await runBounded(
    process.execPath,
    ["-e", "process.on('SIGTERM',()=>{}); setInterval(()=>{},1000); console.log('ready')"],
    {
      cwd: process.cwd(),
      env: {},
      signal: AbortSignal.timeout(500),
      timeoutMs: 5000,
      graceMs: 100,
    },
  );
  assert.equal(result.stopped, true);
  assert.equal(result.status, null);
  assert.match(result.output, /ready/);
});

test("normal process preserves its actual exit code", async () => {
  const result = await runBounded(process.execPath, ["-e", "process.exitCode=7"], {
    cwd: process.cwd(),
    env: {},
    signal: new AbortController().signal,
    timeoutMs: 5000,
  });
  assert.equal(result.status, 7);
  assert.equal(result.stopped, false);
});

test("a TERM-resistant descendant is stopped even after its group leader exits normally", async (context) => {
  const descendant = "process.on('SIGTERM',()=>{}); console.log('ready'); setInterval(()=>{},1000)";
  const leader = `
    const {spawn} = require('node:child_process');
    const child = spawn(process.execPath, ['-e', ${JSON.stringify(descendant)}], {
      stdio: ['ignore', 'pipe', 'ignore']
    });
    child.stdout.once('data', () => {
      console.log(child.pid);
      child.unref();
      process.exit(0);
    });
  `;
  const result = await runBounded(process.execPath, ["-e", leader], {
    cwd: process.cwd(),
    env: {},
    signal: new AbortController().signal,
    timeoutMs: 5000,
    graceMs: 100,
  });
  const pid = Number(result.output.trim());
  assert.ok(Number.isSafeInteger(pid) && pid > 1, "Owned descendant reported its PID");
  async function isRunning() {
    try {
      const stat = await readFile(`/proc/${pid}/stat`, "utf8");
      // A killed orphan may briefly remain a zombie until init reaps it.
      return !/^\d+ \([^)]*\) [ZX] /.test(stat);
    } catch (error) {
      if (["ENOENT", "ESRCH"].includes((error as NodeJS.ErrnoException).code ?? "")) return false;
      throw error;
    }
  }
  context.after(async () => {
    if (await isRunning()) process.kill(pid, "SIGKILL");
  });
  const until = Date.now() + 1000;
  while ((await isRunning()) && Date.now() < until) await delay(20);
  assert.equal(await isRunning(), false, "No running descendant survives leader exit");
  assert.equal(result.status, 0);
  assert.equal(result.stopped, false);
});
