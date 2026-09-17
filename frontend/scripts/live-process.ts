import { spawn } from "node:child_process";

/** Own a Linux process group so timeout/interruption also stops fixture descendants. */
export async function runBounded(
  command: string,
  args: string[],
  options: {
    cwd: string;
    env: NodeJS.ProcessEnv;
    signal: AbortSignal;
    timeoutMs: number;
    graceMs?: number;
  },
) {
  const child = spawn(command, args, {
    cwd: options.cwd,
    env: options.env,
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  });
  const pid = child.pid;
  let output = "";
  let stopped = false;
  let excessive = false;
  let killTimer: ReturnType<typeof setTimeout> | undefined;
  const groupSignal = (signal: NodeJS.Signals | 0) => {
    if (!pid) return false;
    try {
      process.kill(-pid, signal);
      return true;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ESRCH") return false;
      throw error;
    }
  };
  const cancel = () => {
    if (stopped) return;
    stopped = true;
    groupSignal("SIGTERM");
    killTimer = setTimeout(() => groupSignal("SIGKILL"), options.graceMs ?? 7000);
  };
  const collect = (chunk: Buffer) => {
    if (output.length + chunk.length > 2 * 1024 * 1024) {
      excessive = true;
      cancel();
    } else output += chunk.toString();
  };
  child.stdout.on("data", collect);
  child.stderr.on("data", collect);
  const timeout = setTimeout(cancel, options.timeoutMs);
  options.signal.addEventListener("abort", cancel, { once: true });
  if (options.signal.aborted) cancel();
  try {
    const status = await new Promise<number | null>((resolve, reject) => {
      child.once("error", () => reject(new Error("Test process could not start")));
      child.once("close", resolve);
    });
    return { status, output, stopped, excessive };
  } finally {
    clearTimeout(timeout);
    options.signal.removeEventListener("abort", cancel);
    // The group can outlive its leader. Always reap remaining descendants;
    // do not clear escalation merely because the Playwright parent exited.
    if (groupSignal(0)) {
      groupSignal("SIGTERM");
      const until = Date.now() + (options.graceMs ?? 7000);
      while (groupSignal(0) && Date.now() < until)
        await new Promise((resolve) => setTimeout(resolve, 50));
      groupSignal("SIGKILL");
    }
    clearTimeout(killTimer);
  }
}
