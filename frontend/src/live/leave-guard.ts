export const draftLeaveWarning =
  "Leave this Task edit? Local changes and the save retry key will be lost. An in-flight or unknown save may still have been applied by Core; leaving does not undo it.";

/** One active Task form per connection; no edit contents or keys enter browser storage. */
export function createLeaveGuard(confirmLeave: (message: string) => boolean = window.confirm) {
  const checks = new Set<() => boolean>();
  return {
    register(check: () => boolean) {
      checks.add(check);
      return () => {
        checks.delete(check);
      };
    },
    canLeave() {
      return !Array.from(checks).some((check) => check()) || confirmLeave(draftLeaveWarning);
    },
  };
}
export type LeaveGuard = ReturnType<typeof createLeaveGuard>;
