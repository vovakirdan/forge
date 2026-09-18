import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { TaskBrowser } from "./TaskBrowser.tsx";
import { RunBrowser } from "./RunBrowser.tsx";
import { PipelineBrowser } from "./PipelineBrowser.tsx";
import { prepareSectionChange } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";

export function ProjectReads(scope: ProjectReadScope) {
  const queries = useQueryClient();
  const [section, setSection] = useState<"tasks" | "runs" | "pipelines">("tasks");
  function select(next: "tasks" | "runs" | "pipelines") {
    if (next === section) return;
    if (!scope.leaveGuard.canLeave()) return;
    prepareSectionChange(queries, scope.generation, scope.projectId);
    setSection(next);
  }
  return (
    <>
      <nav aria-label="Project sections" className="flex flex-wrap gap-2">
        <Button
          variant={section === "tasks" ? "default" : "outline"}
          aria-pressed={section === "tasks"}
          onClick={() => select("tasks")}
        >
          Tasks
        </Button>
        <Button
          variant={section === "runs" ? "default" : "outline"}
          aria-pressed={section === "runs"}
          onClick={() => select("runs")}
        >
          Runs
        </Button>
        <Button
          variant={section === "pipelines" ? "default" : "outline"}
          aria-pressed={section === "pipelines"}
          onClick={() => select("pipelines")}
        >
          Pipeline versions
        </Button>
      </nav>
      {section === "tasks" && <TaskBrowser {...scope} />}
      {section === "runs" && <RunBrowser {...scope} />}
      {section === "pipelines" && <PipelineBrowser {...scope} />}
    </>
  );
}
