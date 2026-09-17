import type { FullResult, Reporter, TestCase, TestResult } from "@playwright/test/reporter";

/** An intended assertion failure must not hide another fixture/cleanup failure. */
export function classifyProbeErrors(messages: readonly string[]) {
  const expected = messages.filter((message) =>
    /^(?:Error: )?Intentional probe \[redacted\] \[redacted\]$/.test(message),
  ).length;
  return { expected, unexpected: messages.length - expected };
}

export default class FailureProbeReporter implements Reporter {
  private tests = 0;
  private expected = 0;
  private unexpected = 0;
  onTestEnd(_test: TestCase, result: TestResult) {
    this.tests += 1;
    const counts = classifyProbeErrors(result.errors.map((error) => error.message ?? ""));
    this.expected += counts.expected;
    this.unexpected += counts.unexpected + (result.status === "failed" ? 0 : 1);
  }
  onError() {
    this.unexpected += 1;
  }
  onEnd(result: FullResult) {
    console.log(
      `FORGE_FAILURE_PROBE ${JSON.stringify({ tests: this.tests, expected: this.expected, unexpected: this.unexpected, status: result.status })}`,
    );
  }
}
