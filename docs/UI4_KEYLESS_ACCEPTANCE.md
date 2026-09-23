# UI4 keyless browser pagination acceptance

## Method

Run `bash scripts/test-ui-perf.sh` from the repository root. The script builds the live UI, CLI, gateway, and a separate Core fixture, then runs `frontend/tests/perf/keyless.spec.ts` in Chromium. The fixture creates one private Project with one Pipeline, 1,000 draft Tasks, and 20 Employees through Core named commands. It uses local PostgreSQL and NATS. No provider credentials or inference are used.

The browser logs in through the owner CLI, opens that Project, traverses all 50 Task pages, returns one page by keyboard, and opens the Team page. The test parses every observed Task and Employee list response and requires at most 20 items and 64 KiB per response. It asserts the last Task page and the single Employee page have no next page. The test process stops the gateway and fixture after completion. This fixture is separate from the normal live browser suite.

## Local result, 2026-09-23

`bash scripts/test-ui-perf.sh`: **1 passed in 28.8 s of Playwright execution** after builds. The isolated Project contained exactly 1,000 Tasks and 20 Employees. Chromium observed 50 Task list responses and one Employee list response. Largest Task page: 5,844 bytes; largest Employee page: 2,911 bytes. The 49 forward Task page transitions took 6,049 ms in this local run. These are measurements, not a fixed latency guarantee. An earlier direct Playwright run also passed with 5,748 ms for those transitions. The first script attempt failed before fixture startup due to a wrong relative path in the new test; that path was corrected before the passing runs.

This proves bounded real Core → gateway → browser pagination for this local keyless fixture. It does not prove provider execution, production hardware performance, or behavior of other Project sizes and data shapes.
