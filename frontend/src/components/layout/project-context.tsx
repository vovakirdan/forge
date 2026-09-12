import { createContext, useContext, useMemo, useState, type ReactNode } from "react";

interface Ctx {
  projectId: string;
  setProjectId: (id: string) => void;
  commandOpen: boolean;
  setCommandOpen: (v: boolean) => void;
}

const ProjectContext = createContext<Ctx | null>(null);

export function ProjectProvider({ children }: { children: ReactNode }) {
  const [projectId, setProjectId] = useState("prj-surge");
  const [commandOpen, setCommandOpen] = useState(false);
  const value = useMemo(
    () => ({ projectId, setProjectId, commandOpen, setCommandOpen }),
    [projectId, commandOpen],
  );
  return <ProjectContext.Provider value={value}>{children}</ProjectContext.Provider>;
}

export function useProject() {
  const ctx = useContext(ProjectContext);
  if (!ctx) throw new Error("useProject must be used inside ProjectProvider");
  return ctx;
}
