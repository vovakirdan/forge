# UI4 provider live acceptance proposal

**Status:** operator approved the exact allowance on 2026-09-23. The isolated provider run and browser observation completed on 2026-09-23; evidence and limits are below.

## Exact allowance

- Model: Codex `gpt-5.6-luna`, using the owner's existing local auth file. No model substitution.
- Isolated fresh `/tmp` playground and M3-style private Core/session database; no existing Project or repository is modified.
- At most one active Run. Expect three provider Runs (onboarding, Task, summary); request stop when six Runs are observed. An extra Run can occur between polls, so the guard is not a hard billing cap.
- Task Run timeout 600 seconds; SystemJob Run timeout 300 seconds; total session deadline 1,800 seconds after service preparation.
- Startup prints the chosen model, auth source and limits. The operator approved this exact allowance. No provider token, login code or raw evidence body is written to the UI report.

## Browser evidence to collect

Run the existing isolated M3 workflow and connect a freshly built owner gateway to that session's Core socket. Use a one-time owner login code. Observe the same Project in Control Room while the workflow runs: onboarding gate and SystemJob, Employee profile, Task lifecycle and pinned Pipeline, Run purpose/state/context/evidence receipts, immediate handoff, canonical Knowledge versus derived memory, activity reconnect, and Project stop. Record canonical Project/Task/Run IDs, browser assertions and actual cleanup/quiescence evidence. Compare displayed facts with Core reads and the M3 session report. No success is inferred from a green UI badge or from derived summary alone.

The keyless browser suite separately proves command confirmations, scope isolation, revision refusals, replay and failure handling. This provider run checks integration with real execution and the resulting UI reads. It does not prove every provider lane or production hardware.

## Execution boundary

Before spending allowance, finish UI0–UI4 keyless gates and build the exact revision being tested. Reuse the M3 launcher limits and owned-session cleanup; keep the UI gateway and browser observer scoped to that session. On completion or timeout, request Project stop, check physical quiescence, stop the owner gateway, and retain a bounded report under the private session evidence directory. Do not delete unrelated volumes, repositories or credentials.

## Local result, 2026-09-23

`just ui-test-live` passed 177/177 real Core → gateway → Chromium scenarios, then the intentional failure probe passed the secret scan with 596 issued values and no leaks. The separate 1,000 Task / 20 Employee page test passed as recorded in [UI4_KEYLESS_ACCEPTANCE.md](UI4_KEYLESS_ACCEPTANCE.md).

The first isolated M3 attempt stopped in the index phase after its post-restart search returned zero results. Its retained report is `/tmp/fm3.aIw0OeBY/evidence/report.json`; Core recorded zero Runs and cleanup succeeded. A fresh keyless `index` run then passed restart, outage, fallback and catch-up (`/tmp/fm3.L45Zv3N7/evidence/report.json`). The approved provider allowance was spent only in the subsequent fresh run, with the same model and limits.

The provider run passed at `/tmp/fm3.K41JmdoC/evidence/report.json`. The browser observer's safe report is `/tmp/fm3.K41JmdoC/evidence/ui-observer.json`. Project `01a0ce6d-0ae1-7ba1-964c-1af104915e4f`, Employee `01a0ce6d-72b3-75e0-a5de-f8b987d28c42`, Task `01a0ce6d-f4a7-7af2-8ea5-cde9297f42a0` and pinned Pipeline version `01a0ce6d-73a1-7691-97c3-1652c4bec0fe` were observed in the browser. Onboarding reached `completed` and Task reached `done`. The observer saw three distinct Run list and detail cards: onboarding `01a0ce6d-7dfe-7ae2-a766-5e6918007ec4`, Task stage `01a0ce6d-f56a-7d20-ac1b-263d622a7244`, and summarization `01a0ce6f-727c-7cf1-8b78-04d5eafe304b`. Core confirms all three observed states `stopped`; the taskless jobs have no Task or Employee ownership. The Task Run had safe context coordinates available; the two SystemJob Runs did not expose task-stage coordinates. The Run evidence receipt read returned zero for these Runs, which does not negate the separate Task artifact.

Core Task readback contains one `stage_evidence` artifact with `producer=employee_run` and `accepted_as_outcome=true`. The accepted Task summary has three source refs (artifact and two events) and event coverage 23–38. After stop, the Run count stayed three during six seconds of observation; the launcher reported `cleanup_failed=false` and no running session containers. The final SystemJob list includes a pending second summarization generation requested by the stop proof; this did not dispatch a fourth Run while stopped.

The observer recorded two browser console errors during the session shutdown window, without raw messages by design. They did not prevent its safe UI assertions or the canonical M3 result, but this run cannot claim a zero-error console across shutdown. This evidence proves this local Codex lane and UI read path; it does not prove other provider lanes, clean-host installation, every optional hook/Git recovery path, or semantic truth of the model's summary.

The bounded hook-invocation and SystemJob-attempt read panels were completed after this provider session. Their Core/storage and frontend contracts have separate keyless checks; the provider observation above is not evidence that those two panels displayed populated histories in Chromium.

After those panels landed, the full local keyless suite passed again with 178/178 browser scenarios. Its intentional failure probe passed the artifact/output secret scan with 598 issued values and no leaks. This later keyless result does not change the narrower provider observation above.
