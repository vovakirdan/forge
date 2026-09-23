import assert from "node:assert/strict";
import { test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { PipelineVersionViewSchema } from "../../src/contracts/pipeline.ts";
import { analysisPipelineFixture, deliveryPipelineFixture } from "../../src/contracts/fixtures.ts";
import { PipelineDefinition } from "../../src/live/PipelineDefinition.tsx";
import { PipelineStageInspector } from "../../src/live/PipelineStageInspector.tsx";

test("graph renders arbitrary stages, a rework cycle, and terminal routes", () => {
  const pipeline = PipelineVersionViewSchema.parse(deliveryPipelineFixture);
  const html = renderToStaticMarkup(<PipelineDefinition pipeline={pipeline} />);

  assert.match(html, /Stage graph/);
  assert.match(html, /View graph stage work/);
  assert.match(html, /Follow rework to stage work/);
  assert.match(html, /Follow stale_base to stage work/);
  assert.match(html, /done \(terminal\)/);
  assert.match(html, /cancelled \(terminal\)/);
  assert.match(html, /Entry stage/);
  assert.match(html, /at least 1 from current_stage/);
});

test("graph shows a Pipeline without review, QA, Git action or hooks", () => {
  const pipeline = PipelineVersionViewSchema.parse(analysisPipelineFixture);
  const html = renderToStaticMarkup(<PipelineDefinition pipeline={pipeline} />);

  assert.match(html, /View graph stage explore/);
  assert.match(html, /Follow more_research to stage explore/);
  assert.doesNotMatch(html, /View graph stage review/);
  assert.doesNotMatch(html, /Passed outcome/);
});

test("hook stage reports configured mapping without inventing a run result", () => {
  const pipeline = PipelineVersionViewSchema.parse({
    ...deliveryPipelineFixture,
    stages: deliveryPipelineFixture.stages.map((stage) =>
      stage.id === "integrate"
        ? {
            ...stage,
            system_action: {
              kind: "project_hook",
              hook_version_id: "01a00000-0000-7000-8000-000000000011",
              outcomes: {
                passed: "passed",
                failed: "failed",
                timed_out: "timed_out",
                skipped: "skipped",
              },
            },
          }
        : stage,
    ),
  });
  const stage = pipeline.stages.find((item) => item.id === "integrate");
  assert.ok(stage);
  const html = renderToStaticMarkup(<PipelineStageInspector stage={stage} pipeline={pipeline} />);

  assert.match(html, /optional hook version/);
  assert.match(html, /not reported by this version definition/);
  assert.match(html, /Skipped outcome/);
  assert.doesNotMatch(html, /Hook passed/);
});
