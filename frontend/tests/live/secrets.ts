import { createConnection } from "node:net";
import type { TestInfo } from "@playwright/test";

/** A private test-only UDS sends scan markers to the parent process, not stdout. */
export async function registerSecrets(values: string[]) {
  const path = process.env["FORGE_UI_TEST_REGISTRY"];
  if (!path) throw new Error("Run this suite through just ui-test-live");
  await new Promise<void>((resolve, reject) => {
    const socket = createConnection(path);
    socket.setTimeout(2000, () => socket.destroy(new Error("Marker registration timed out")));
    socket.once("error", () => reject(new Error("Private marker registration failed")));
    socket.once("connect", () => socket.end(`${JSON.stringify(values)}\n`));
    socket.once("close", (hadError) => {
      if (!hadError) resolve();
    });
  });
}

export function redact(text: string, secrets: Iterable<string>) {
  let result = text;
  for (const value of secrets) if (value) result = result.replaceAll(value, "[redacted]");
  return result;
}

export function sanitizeErrors(info: TestInfo, secrets: Iterable<string>) {
  for (const error of info.errors) {
    if (error.message) error.message = redact(error.message, secrets);
    if (error.stack) error.stack = redact(error.stack, secrets);
    if ("snippet" in error && typeof error.snippet === "string")
      error.snippet = redact(error.snippet, secrets);
  }
}
