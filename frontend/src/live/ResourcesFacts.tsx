import type { z } from "zod";
import type { ProjectResourcesSchema } from "../contracts/resources.ts";
import { Field } from "./Field.tsx";

type Resources = z.infer<typeof ProjectResourcesSchema>;

export function ResourcesFacts({ snapshot }: { snapshot: Resources }) {
  return (
    <div className="space-y-4">
      <p className="text-xs text-muted-foreground">
        Admission limits are startup configuration. Occupied slots are current Core evidence; usage
        and cost measurements are unavailable.
      </p>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
        <Field label="Host occupied">
          {snapshot.occupancy.host_runs} / {snapshot.policy.host_max_runs}
        </Field>
        <Field label="Project occupied">
          {snapshot.occupancy.project_runs} / {snapshot.policy.project_max_runs}
        </Field>
        <Field label="Credential account cap">{snapshot.policy.credential_account_max_runs}</Field>
        <Field label="Credential account occupied">Unknown</Field>
        <Field label="Policy revision">{snapshot.policy.revision}</Field>
        <Field label="Policy updated">{snapshot.policy.updated_at}</Field>
        <Field label="Observed">{snapshot.observed_at}</Field>
      </dl>
    </div>
  );
}
