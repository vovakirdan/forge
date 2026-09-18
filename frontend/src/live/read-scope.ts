import type { LiveApi } from "./api.ts";
import type { LiveSession } from "./session.ts";
import type { LeaveGuard } from "./leave-guard.ts";

export type ProjectReadScope = {
  api: LiveApi;
  session: LiveSession;
  generation: number;
  projectId: string;
  leaveGuard: LeaveGuard;
};
