import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { once } from "node:events";

const children = new Set<ChildProcessWithoutNullStreams>();
const READINESS_LIMIT = 12 * 1024;
for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.once(signal, () => {
    void Promise.all([...children].map(stop)).finally(() => process.exit(1));
  });
}
process.once("exit", () => {
  for (const child of children) child.kill("SIGTERM");
});

export function launch(command: string, args: string[], cwd: string) {
  const child = spawn(command, args, {
    cwd,
    // In particular: no DB, provider or owner auth environment for gateway/CLI.
    env: { PATH: process.env["PATH"] ?? "/usr/bin:/bin", LANG: "C.UTF-8" },
    stdio: "pipe",
  });
  children.add(child);
  child.once("exit", () => children.delete(child));
  child.once("error", () => children.delete(child));
  return child;
}

export async function firstLine(child: ChildProcessWithoutNullStreams): Promise<string> {
  return new Promise((resolve, reject) => {
    let output = "";
    const timer = setTimeout(() => finish(new Error("Fixture startup timed out")), 40_000);
    const onData = (chunk: Buffer) => {
      output += chunk.toString();
      if (output.length > READINESS_LIMIT)
        finish(new Error("Fixture startup exceeded output bound"));
      else if (output.includes("\n")) finish(undefined, output.split("\n")[0]);
    };
    const onExit = () => finish(new Error("Fixture exited before readiness"));
    function finish(error?: Error, line?: string) {
      clearTimeout(timer);
      child.stdout.off("data", onData);
      child.off("exit", onExit);
      child.off("error", onExit);
      if (error) reject(error);
      else resolve(line ?? "");
    }
    child.stdout.on("data", onData);
    child.once("exit", onExit);
    child.once("error", onExit);
  });
}

export async function stop(child: ChildProcessWithoutNullStreams) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  const exited = once(child, "exit");
  child.stdin.end();
  child.kill("SIGTERM");
  const timer = setTimeout(() => child.kill("SIGKILL"), 7000);
  try {
    await exited;
  } finally {
    clearTimeout(timer);
  }
}

export function quoteShell(value: string) {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

/** Captures the controlling terminal output in memory, never a log or fixture. */
export async function terminalLogin(binary: string, control: string, cwd: string) {
  const command = `${quoteShell(binary)} ui login --control-socket ${quoteShell(control)}`;
  const child = launch("script", ["--quiet", "--return", "--command", command, "/dev/null"], cwd);
  let output = "";
  let excessive = false;
  child.stdout.on("data", (chunk: Buffer) => {
    if (output.length + chunk.length > 8192) {
      excessive = true;
      child.kill("SIGKILL");
    } else output += chunk.toString();
  });
  child.stderr.resume();
  const timer = setTimeout(() => child.kill("SIGKILL"), 10_000);
  try {
    const [status] = await once(child, "exit");
    if (status !== 0 || excessive) throw new Error("Owner terminal login failed");
    const code = /^Code: ([a-f0-9]{64})\r?$/m.exec(output)?.[1];
    const origin = /^Origin: (http:\/\/127\.0\.0\.1:\d+)\r?$/m.exec(output)?.[1];
    if (!code || !origin) throw new Error("Owner terminal login returned an invalid envelope");
    return { code, origin };
  } finally {
    clearTimeout(timer);
    output = "";
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
  }
}
