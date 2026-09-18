import assert from "node:assert/strict";
import { test } from "node:test";
import {
  AmendDraftRequestSchema,
  AmendDraftReceiptSchema,
  DraftTitleSchema,
  DraftDescriptionSchema,
} from "./amend-draft.ts";
import { ids } from "./fixtures.ts";

const request = {
  project_id: ids.project,
  expected_revision: 3,
  payload: { task_id: ids.task, expected_task_revision: 7, patch: { title: "New title" } },
};
const receipt = {
  command_id: ids.actor,
  status: "applied",
  project_revision: 4,
  event_ids: [ids.artifact],
  resource: { kind: "task", id: ids.task },
};

test("draft text limits count Unicode scalars and match Rust whitespace without trimming", () => {
  for (const invalid of ["", " \t\n", "\u0085", "\u2000", "😀".repeat(241), "\uD800", "a\uDC00b"])
    assert.equal(DraftTitleSchema.safeParse(invalid).success, false, JSON.stringify(invalid));
  for (const valid of [" padded ", "😀".repeat(240), "\uFEFF"])
    assert.equal(DraftTitleSchema.parse(valid), valid);
  assert.equal(DraftDescriptionSchema.parse(""), "");
  assert.equal(DraftDescriptionSchema.safeParse("😀".repeat(50_000)).success, true);
  assert.equal(DraftDescriptionSchema.safeParse("😀".repeat(50_001)).success, false);
  assert.equal(DraftDescriptionSchema.safeParse("\uDFFF").success, false);
});

test("draft request closes all objects to unknown fields and refuses empty patches", () => {
  assert.deepEqual(AmendDraftRequestSchema.parse(request), request);
  for (const invalid of [
    { ...request, actor: ids.actor },
    { ...request, expected_revision: Number.MAX_SAFE_INTEGER + 1 },
    { ...request, expected_revision: Number.MAX_SAFE_INTEGER },
    {
      ...request,
      payload: { ...request.payload, expected_task_revision: Number.MAX_SAFE_INTEGER },
    },
    { ...request, payload: { ...request.payload, approval: true } },
    { ...request, payload: { ...request.payload, patch: {} } },
    { ...request, payload: { ...request.payload, patch: { title: "fine", priority: "high" } } },
    { ...request, payload: { ...request.payload, patch: { description: null } } },
  ])
    assert.equal(AmendDraftRequestSchema.safeParse(invalid).success, false);
});

test("draft receipts require known status, exact Task resource and valid event identities", () => {
  assert.deepEqual(AmendDraftReceiptSchema.parse(receipt), receipt);
  assert.equal(
    AmendDraftReceiptSchema.parse({ ...receipt, status: "replayed" }).status,
    "replayed",
  );
  for (const invalid of [
    { ...receipt, status: "accepted" },
    { ...receipt, command_id: "bad" },
    { ...receipt, event_ids: ["bad"] },
    { ...receipt, event_ids: [] },
    { ...receipt, resource: { kind: "project", id: ids.project } },
    { ...receipt, resource: undefined },
    { ...receipt, debug: "private" },
  ])
    assert.equal(AmendDraftReceiptSchema.safeParse(invalid).success, false);
});
