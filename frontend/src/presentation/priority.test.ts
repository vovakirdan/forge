import assert from "node:assert/strict";
import { test } from "node:test";
import { PrioritySchemeViewSchema } from "../contracts/priority-scheme.ts";
import { priorityLabel } from "./priority.ts";
import { freezeFixture } from "./test-helpers.ts";

const scheme = freezeFixture(
  PrioritySchemeViewSchema.parse({
    project_id: "01a08a41-977c-7d23-a347-22c7afcfb653",
    project_revision: 7,
    default_level_id: "owner_custom",
    levels: [
      { id: "owner_custom", display_name: "По возможности", rank: -10, retired: false },
      { id: "old", display_name: "Historical", rank: -10, retired: true },
      { id: "unsafe_text", display_name: "<script>alert(1)</script>", rank: -1, retired: false },
    ],
  }),
);

test("priority presentation resolves arbitrary labels by exact ID without changing wire values", () => {
  const catalog = { status: "loaded", scheme, stale: false } as const;
  assert.equal(priorityLabel("owner_custom", catalog), "По возможности");
  assert.equal(priorityLabel("old", catalog), "Historical (retired)");
  assert.equal(priorityLabel("unsafe_text", catalog), "<script>alert(1)</script>");
  assert.equal(scheme.levels[1]?.display_name, "Historical");
  assert.deepEqual(
    scheme.levels.map((level) => level.rank),
    [-10, -10, -1],
  );
});

test("unknown, loading and failed catalogs never substitute the scheme default", () => {
  assert.equal(
    priorityLabel("missing", { status: "loaded", scheme, stale: false }),
    "Name unavailable (ID not in catalog)",
  );
  assert.equal(
    priorityLabel("OWNER_CUSTOM", { status: "loaded", scheme, stale: false }),
    "Name unavailable (ID not in catalog)",
  );
  assert.equal(
    priorityLabel("owner_custom", { status: "loading" }),
    "Name unavailable (loading priorities)",
  );
  assert.equal(
    priorityLabel("owner_custom", { status: "unavailable" }),
    "Name unavailable (catalog unavailable)",
  );
});

test("stale catalogs stay explicit for active, retired and absent Task priorities", () => {
  const catalog = { status: "loaded", scheme, stale: true } as const;
  assert.equal(priorityLabel("owner_custom", catalog), "По возможности (stale catalog)");
  assert.equal(priorityLabel("old", catalog), "Historical (retired) (stale catalog)");
  assert.equal(
    priorityLabel("missing", catalog),
    "Name unavailable (ID not in catalog) (stale catalog)",
  );
});
