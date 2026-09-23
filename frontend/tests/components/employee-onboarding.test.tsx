import assert from "node:assert/strict";
import { test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { EmployeeOnboardingStatusSchema } from "../../src/contracts/system-jobs.ts";
import { EmployeeOnboardingFacts } from "../../src/live/EmployeeOnboardingPanel.tsx";

const base = {
  employee_id: "01988000-0000-7000-8000-000000000002",
  revision: 2,
  job_id: null,
  receipt: null,
};

test("onboarding shows pending, skipped and legacy bypass without completed claim", () => {
  for (const state of ["pending", "skipped", "legacy_bypass"] as const) {
    const value = EmployeeOnboardingStatusSchema.parse({ ...base, state });
    const html = renderToStaticMarkup(<EmployeeOnboardingFacts value={value} />);
    assert.match(html, new RegExp(state));
    assert.doesNotMatch(html, /Completed records a familiarity receipt/);
  }
  const completed = EmployeeOnboardingStatusSchema.parse({
    ...base,
    state: "completed",
    receipt: { at: "2026-09-23T12:00:00Z" },
  });
  const html = renderToStaticMarkup(<EmployeeOnboardingFacts value={completed} />);
  assert.match(html, /Completed records a familiarity receipt/);
  assert.match(html, /does not prove comprehension/);
});
