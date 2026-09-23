export const draftLeaveWarning =
  "Leave this Task edit? Local changes and the save retry key will be lost. An in-flight or unknown save may still have been applied by Core; leaving does not undo it.";

/** One active Task form per connection; no edit contents or keys enter browser storage. */
export function createLeaveGuard(confirmLeave: (message: string) => boolean = window.confirm) {
  const checks = new Set<{ check: () => boolean; message: string }>();
  return {
    hasPending() {
      return Array.from(checks).some((entry) => entry.check());
    },
    register(check: () => boolean, message = draftLeaveWarning) {
      const entry = { check, message };
      checks.add(entry);
      return () => {
        checks.delete(entry);
      };
    },
    canLeave() {
      const warning = Array.from(checks).find((entry) => entry.check())?.message;
      return warning === undefined || confirmLeave(warning);
    },
  };
}
export type LeaveGuard = ReturnType<typeof createLeaveGuard>;
