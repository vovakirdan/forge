import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import type { EmployeeOnboardingStatus } from "../contracts/system-jobs.ts";
import { describeApiError } from "./api.ts";
import { Field } from "./Field.tsx";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { SystemJobCommandButton } from "./SystemJobCommandButton.tsx";

export function EmployeeOnboardingPanel({
  scope,
  employeeId,
}: {
  scope: ProjectReadScope;
  employeeId: string;
}) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const [reason, setReason] = useState("");
  useEffect(() => {
    if (!reason) return;
    return leaveGuard.register(
      () => true,
      "Leave onboarding management? The unsaved skip reason will be lost.",
    );
  }, [leaveGuard, reason]);
  const key = useMemo(
    () => readKeys.employeeOnboarding(generation, projectId, employeeId),
    [generation, projectId, employeeId],
  );
  const status = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.employeeOnboarding(projectId, employeeId, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  const value = status.data;
  return (
    <section aria-label="Employee onboarding" className="space-y-3 border-t border-border pt-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h3 className="font-medium">Onboarding</h3>
        <Button
          variant="outline"
          disabled={status.isFetching}
          onClick={() => void status.refetch()}
        >
          Refresh onboarding
        </Button>
      </div>
      {status.isPending && <p role="status">Loading onboarding status…</p>}
      {status.isError && (
        <p role="alert">
          {value ? "Showing stale onboarding status. " : ""}
          {describeApiError(status.error, "Employee")}
        </p>
      )}
      {value && <EmployeeOnboardingFacts value={value} />}
      {value && (
        <div className="space-y-3 border-t border-border pt-3">
          <p className="text-xs text-muted-foreground">
            Requesting onboarding creates an audited System Job. A queued job is not a completed
            familiarity receipt.
          </p>
          <SystemJobCommandButton
            {...scope}
            action="request_employee_onboarding"
            label="Request Employee onboarding"
            payload={() => ({ employee_id: employeeId })}
            onApplied={() => void status.refetch()}
          />
          {value.state === "pending" && (
            <>
              <label className="block text-sm">
                Reason for skipping onboarding
                <textarea
                  value={reason}
                  onChange={(event) => setReason(event.target.value)}
                  rows={3}
                  className="mt-1 w-full rounded border border-border bg-background p-2 text-sm"
                />
              </label>
              <SystemJobCommandButton
                {...scope}
                action="skip_employee_onboarding"
                label="Skip Employee onboarding"
                payload={() => ({ employee_id: employeeId, reason })}
                onApplied={() => {
                  setReason("");
                  void status.refetch();
                }}
              />
            </>
          )}
        </div>
      )}
    </section>
  );
}

export function EmployeeOnboardingFacts({ value }: { value: EmployeeOnboardingStatus }) {
  return (
    <>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
        <Field label="State">{value.state}</Field>
        <Field label="Revision">{value.revision}</Field>
        <Field label="Job ID">{value.job_id ?? "No job linked"}</Field>
        <Field label="Receipt">{value.receipt === null ? "Not recorded" : "Recorded"}</Field>
      </dl>
      {value.state === "completed" && (
        <p className="text-xs text-muted-foreground">
          Completed records a familiarity receipt. It does not prove comprehension or a retained
          provider session.
        </p>
      )}
      {value.state === "skipped" && (
        <p className="text-xs text-muted-foreground">
          Onboarding was explicitly skipped; this is not a completed familiarity receipt.
        </p>
      )}
      {value.state === "legacy_bypass" && (
        <p className="text-xs text-muted-foreground">
          This existing Employee bypassed onboarding during migration. No completion is implied.
        </p>
      )}
      {value.state === "pending" && (
        <p className="text-xs text-muted-foreground">
          Onboarding is pending. A linked job does not prove that a provider Run has started.
        </p>
      )}
    </>
  );
}
