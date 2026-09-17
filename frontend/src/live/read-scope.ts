import type { LiveApi } from "./api.ts";
import type { LiveSession } from "./session.ts";

export type ProjectReadScope = {
  api: LiveApi;
  session: LiveSession;
  generation: number;
  projectId: string;
};
