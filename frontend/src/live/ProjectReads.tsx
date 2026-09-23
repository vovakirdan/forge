import { TaskBrowser } from "./TaskBrowser.tsx";
import { RunBrowser } from "./RunBrowser.tsx";
import { PipelineBrowser } from "./PipelineBrowser.tsx";
import { TeamBrowser } from "./TeamBrowser.tsx";
import { KnowledgeBrowser } from "./KnowledgeBrowser.tsx";
import { SystemJobsBrowser } from "./SystemJobsBrowser.tsx";
import { ResourcesBrowser } from "./ResourcesBrowser.tsx";
import { ManagementBrowser } from "./ManagementBrowser.tsx";
import type { ProjectReadScope } from "./read-scope.ts";

export type ProjectSection =
  | "tasks"
  | "runs"
  | "pipelines"
  | "team"
  | "activity"
  | "knowledge"
  | "system-jobs"
  | "resources"
  | "management";

export function ProjectReads(scope: ProjectReadScope & { section: ProjectSection }) {
  const { section } = scope;
  return (
    <>
      {section === "tasks" && <TaskBrowser {...scope} />}
      {section === "team" && <TeamBrowser {...scope} />}
      {section === "runs" && <RunBrowser {...scope} />}
      {section === "pipelines" && <PipelineBrowser {...scope} />}
      {section === "knowledge" && <KnowledgeBrowser {...scope} />}
      {section === "system-jobs" && <SystemJobsBrowser {...scope} />}
      {section === "resources" && <ResourcesBrowser {...scope} />}
      {section === "management" && <ManagementBrowser {...scope} />}
    </>
  );
}
