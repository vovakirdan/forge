import assert from "node:assert/strict";
import { test } from "node:test";
import { classifyProbeErrors } from "./live-failure-reporter.ts";

test("only the single exact intended failure is accepted", () => {
  assert.deepEqual(classifyProbeErrors(["Error: Intentional probe [redacted] [redacted]"]), {
    expected: 1,
    unexpected: 0,
  });
  assert.deepEqual(
    classifyProbeErrors([
      "Error: Intentional probe [redacted] [redacted]",
      "Gateway diagnostics exposed session material",
    ]),
    { expected: 1, unexpected: 1 },
  );
  assert.deepEqual(classifyProbeErrors(["Error: cleanup failed"]), { expected: 0, unexpected: 1 });
});
