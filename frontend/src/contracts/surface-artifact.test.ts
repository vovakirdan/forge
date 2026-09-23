import assert from "node:assert/strict";
import { test } from "node:test";
import { FileSnapshotsSchema, GitSourceSettingSchema } from "./surface-artifact.ts";

const project = "01988000-0000-7000-8000-000000000001";
const task = "01988000-0000-7000-8000-000000000002";
const artifact = "01988000-0000-7000-8000-000000000003";
const snapshot = "01988000-0000-7000-8000-000000000004";

test("safe Core snapshot manifest contains only file metadata", () => {
  const page = FileSnapshotsSchema.parse({
    items: [
      {
        id: snapshot,
        artifact_id: artifact,
        task_id: task,
        title: "Result",
        state: "sealed",
        error_code: null,
        created_at: "2026-09-23T10:00:00Z",
        manifest: {
          schema_version: 1,
          project_id: project,
          source_task_id: task,
          files: [
            {
              path: "report.txt",
              size_bytes: 4,
              executable: false,
              sha256: "a".repeat(64),
            },
          ],
        },
      },
    ],
  });
  assert.equal(page.items[0]?.manifest?.files[0]?.path, "report.txt");
  assert.equal(JSON.stringify(page).includes("object_key"), false);
  assert.equal(JSON.stringify(page).includes("raw-key"), false);
  const withUnexpectedKey = structuredClone(page) as unknown as {
    items: { manifest: { files: { object_key?: string }[] } }[];
  };
  withUnexpectedKey.items[0]!.manifest.files[0]!.object_key = "unexpected-storage-coordinate";
  assert.equal(
    JSON.stringify(FileSnapshotsSchema.parse(withUnexpectedKey)).includes(
      "unexpected-storage-coordinate",
    ),
    false,
  );
});

test("pending snapshot cannot present a sealed manifest and source policy stays explicit", () => {
  const base = {
    id: snapshot,
    artifact_id: artifact,
    task_id: task,
    title: "Result",
    state: "pending",
    error_code: null,
    created_at: "2026-09-23T10:00:00Z",
  };
  assert.equal(
    FileSnapshotsSchema.safeParse({ items: [{ ...base, manifest: null }] }).success,
    true,
  );
  assert.equal(
    FileSnapshotsSchema.safeParse({ items: [{ ...base, manifest: {} }] }).success,
    false,
  );
  assert.equal(
    GitSourceSettingSchema.parse({ revision: 1, policy: { mode: "latest_target" } }).policy.mode,
    "latest_target",
  );
});
