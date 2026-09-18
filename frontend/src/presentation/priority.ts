import type { PrioritySchemeView } from "../contracts/priority-scheme.ts";

export type PriorityCatalog =
  | { status: "loading" }
  | { status: "unavailable" }
  | { status: "loaded"; scheme: PrioritySchemeView; stale: boolean };

/** Resolve only the Task's exact ID; the scheme default is not a read fallback. */
export function priorityLabel(id: string, catalog: PriorityCatalog): string {
  if (catalog.status === "loading") return "Name unavailable (loading priorities)";
  if (catalog.status === "unavailable") return "Name unavailable (catalog unavailable)";
  const level = catalog.scheme.levels.find((candidate) => candidate.id === id);
  const label = level
    ? `${level.display_name}${level.retired ? " (retired)" : ""}`
    : "Name unavailable (ID not in catalog)";
  return `${label}${catalog.stale ? " (stale catalog)" : ""}`;
}
