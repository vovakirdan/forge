import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import {
  knowledgeAttempt,
  reserveKnowledgePageId,
  type KnowledgeAction,
  type KnowledgeAttempt,
  type KnowledgePage,
  type KnowledgeReceipt,
} from "../contracts/knowledge.ts";
import { describeApiError, LiveApiError } from "./api.ts";
import { LiveCommandError } from "./command-error.ts";
import { sendKnowledgeCommand } from "./knowledge-command-api.ts";
import { readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type Props = ProjectReadScope & {
  initialPage: KnowledgePage | null;
  onClose: () => void;
  onApplied: (pageId: string) => void;
};
type State =
  | { status: "editing" }
  | { status: "sending" | "unknown"; attempt: KnowledgeAttempt }
  | { status: "refused"; error: LiveCommandError }
  | { status: "accepted"; receipt: KnowledgeReceipt; readError: boolean };

const kindOptions = ["introduction", "architecture", "guide", "policy", "decision"] as const;

function refusal(error: LiveCommandError): string {
  switch (error.kind) {
    case "stale_revision":
      return "Project or page revision changed. Refresh the baseline before choosing an action again.";
    case "idempotency_conflict":
      return "This request key belongs to another command. Refresh before trying again.";
    case "conflict":
      return "This editorial action conflicts with the current page state. Refresh and review the page.";
    case "validation_failed":
      return "Core rejected the page content or current publication state. Refresh and review the fields.";
    case "invalid_request":
      return "The command was refused. Check the fields and refresh before trying again.";
    case "not_found":
      return "The Project, page, or cited source was not found. Refresh before trying again.";
    case "forbidden":
      return "Core refused this editorial action. Refresh before trying again.";
    case "outcome_unknown":
      return "Outcome unknown. Retry the same request to obtain its receipt.";
  }
}

export function KnowledgeCommandPanel({ initialPage, onClose, onApplied, ...scope }: Props) {
  const { api, session, generation, projectId, leaveGuard } = scope;
  const queries = useQueryClient();
  const [reservedId] = useState(() => initialPage?.id ?? reserveKnowledgePageId());
  const [title, setTitle] = useState(initialPage?.content.title ?? "");
  const [markdown, setMarkdown] = useState(initialPage?.content.markdown ?? "");
  const [kind, setKind] = useState<(typeof kindOptions)[number]>(initialPage?.kind ?? "guide");
  const [sourcesText, setSourcesText] = useState(
    JSON.stringify(initialPage?.content.source_refs ?? [], null, 2),
  );
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [state, setState] = useState<State>({ status: "editing" });
  const [acknowledgedRevision, setAcknowledgedRevision] = useState(initialPage?.revision ?? 0);
  const [pendingAction, setPendingAction] = useState<KnowledgeAction | null>(null);
  const [refreshError, setRefreshError] = useState<string | null>(null);
  const lifetime = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const titleRef = useRef<HTMLHeadingElement>(null);
  const confirmRef = useRef<HTMLHeadingElement>(null);
  const baselineKey = useMemo(
    () => ["live", generation, projectId, "knowledge-edit-baseline", reservedId] as const,
    [generation, projectId, reservedId],
  );
  const projectKey = useMemo(
    () => ["project", generation, projectId] as const,
    [generation, projectId],
  );
  const pageKey = useMemo(
    () => readKeys.knowledgePage(generation, projectId, reservedId),
    [generation, projectId, reservedId],
  );
  const baseline = useQuery({
    queryKey: baselineKey,
    queryFn: async ({ signal }) => {
      const [project, page] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, signal)),
        initialPage
          ? session.request(generation, (token) =>
              api.knowledgePage(projectId, reservedId, token, signal),
            )
          : Promise.resolve(null),
      ]);
      return { project, page };
    },
    retry: false,
  });
  useReadLifetime(baselineKey);
  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    titleRef.current?.focus();
    return () => controller.abort();
  }, []);
  useEffect(() => {
    if (pendingAction) confirmRef.current?.focus();
  }, [pendingAction]);
  const guard =
    state.status === "sending" ||
    state.status === "unknown" ||
    title !== (initialPage?.content.title ?? "") ||
    markdown !== (initialPage?.content.markdown ?? "") ||
    kind !== (initialPage?.kind ?? "guide") ||
    sourcesText !== JSON.stringify(initialPage?.content.source_refs ?? [], null, 2);
  useEffect(() => {
    if (!guard) return;
    const unregister = leaveGuard.register(
      () => true,
      "Leave Knowledge editing? Unsaved fields and the retry key will be lost. An in-flight command may still have been applied by Core.",
    );
    function beforeUnload(event: BeforeUnloadEvent) {
      event.preventDefault();
      event.returnValue = "";
    }
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      unregister();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  }, [leaveGuard, guard]);
  function current(controller: AbortController) {
    return !controller.signal.aborted && session.getSnapshot().generation === generation;
  }

  async function readAccepted(receipt: KnowledgeReceipt, controller: AbortController) {
    setState({ status: "accepted", receipt, readError: false });
    try {
      const [project, page] = await Promise.all([
        session.request(generation, (token) => api.project(projectId, token, controller.signal)),
        session.request(generation, (token) =>
          api.knowledgePage(projectId, reservedId, token, controller.signal),
        ),
      ]);
      if (!current(controller)) return;
      if (
        project.revision < receipt.project_revision ||
        page.revision <= (baseline.data?.page?.revision ?? 0)
      )
        throw new Error("Canonical readback did not confirm this revision");
      await Promise.all([
        queries.cancelQueries({ queryKey: projectKey, exact: true }),
        queries.cancelQueries({ queryKey: pageKey, exact: true }),
      ]);
      if (!current(controller)) return;
      queries.setQueryData(projectKey, project);
      queries.setQueryData(pageKey, page);
      void queries.invalidateQueries({
        queryKey: ["live", generation, projectId, "knowledge-pages"],
      });
      void queries.invalidateQueries({
        queryKey: ["live", generation, projectId, "knowledge-history", reservedId],
      });
      onApplied(reservedId);
    } catch {
      if (current(controller)) setState({ status: "accepted", receipt, readError: true });
    }
  }
  async function submit(attempt: KnowledgeAttempt) {
    const controller = lifetime.current;
    if (!controller || inFlight.current) return;
    inFlight.current = true;
    setState({ status: "sending", attempt });
    try {
      const receipt = await session.request(generation, (token) =>
        sendKnowledgeCommand(
          fetch,
          attempt,
          token,
          AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
          () => new LiveApiError("unauthorized"),
        ),
      );
      if (current(controller)) await readAccepted(receipt, controller);
    } catch (error) {
      if (current(controller))
        setState(
          error instanceof LiveCommandError && error.kind !== "outcome_unknown"
            ? { status: "refused", error }
            : { status: "unknown", attempt },
        );
    } finally {
      inFlight.current = false;
    }
  }
  async function refreshBaseline() {
    const controller = lifetime.current;
    if (!controller || baseline.isFetching) return;
    setRefreshError(null);
    const fresh = await baseline.refetch();
    if (!current(controller)) return;
    if (fresh.isError || !fresh.data) {
      setRefreshError(describeApiError(fresh.error));
      return;
    }
    await Promise.all([
      queries.cancelQueries({ queryKey: projectKey, exact: true }),
      queries.cancelQueries({ queryKey: pageKey, exact: true }),
    ]);
    if (!current(controller)) return;
    queries.setQueryData(projectKey, fresh.data.project);
    if (fresh.data.page) queries.setQueryData(pageKey, fresh.data.page);
    setAcknowledgedRevision(fresh.data.page?.revision ?? 0);
    setPendingAction(null);
    setState({ status: "editing" });
  }
  function prepare(action: KnowledgeAction) {
    if (!baseline.data || baseline.isFetching || baseline.isError) return;
    const page = baseline.data.page;
    if ((page?.revision ?? 0) !== acknowledgedRevision) {
      setFieldError(
        "The page changed since this form opened. Refresh Project and page before sending.",
      );
      return;
    }
    const allowed =
      page === null
        ? action === "author_knowledge_page"
        : page.status === "draft"
          ? action === "author_knowledge_page" ||
            action === "publish_knowledge_page" ||
            action === "withdraw_knowledge_page"
          : page.status === "published"
            ? action === "supersede_knowledge_page" || action === "withdraw_knowledge_page"
            : false;
    if (!allowed) return;
    let sources: unknown = [];
    if (action === "author_knowledge_page" || action === "supersede_knowledge_page") {
      try {
        sources = JSON.parse(sourcesText) as unknown;
      } catch {
        setFieldError("Source references must be a JSON array.");
        return;
      }
    }
    if (
      action === "publish_knowledge_page" &&
      page &&
      (title !== page.content.title ||
        markdown !== page.content.markdown ||
        sourcesText !== JSON.stringify(page.content.source_refs, null, 2) ||
        kind !== page.kind)
    ) {
      setFieldError(
        "Save draft edits before publishing. Publication uses the current canonical draft.",
      );
      return;
    }
    try {
      const input = knowledgeAttempt(action, {
        project_id: projectId,
        expected_revision: baseline.data.project.revision,
        payload: {
          page_id: reservedId,
          expected_page_revision: page?.revision ?? 0,
          ...(action === "author_knowledge_page"
            ? { kind, title, markdown, source_refs: sources }
            : {}),
          ...(action === "supersede_knowledge_page"
            ? { title, markdown, source_refs: sources }
            : {}),
        },
      });
      setFieldError(null);
      setPendingAction(null);
      void submit(input);
    } catch {
      setFieldError("Check title, Markdown, source references, and current revisions.");
    }
  }
  const page = baseline.data?.page;
  const editing = state.status === "editing";
  return (
    <section
      aria-label="Edit canonical page"
      className="space-y-4 rounded-xl border border-border bg-card p-6"
    >
      <h3 ref={titleRef} tabIndex={-1} className="font-semibold">
        {initialPage ? "Manage canonical page" : "Create canonical page draft"}
      </h3>
      <p className="text-xs text-muted-foreground">
        Each action creates an immutable revision. A published version cannot be edited in place;
        supersede creates its next published revision. The new draft keeps its reserved page ID
        across retries.
      </p>
      <p className="text-sm">
        Page ID: <span className="font-mono [overflow-wrap:anywhere]">{reservedId}</span>
      </p>
      {baseline.isPending && <p role="status">Loading Project and page revisions…</p>}
      {baseline.isError && <p role="alert">{describeApiError(baseline.error)}</p>}
      {page && (
        <p className="text-sm">
          Current status: {page.status}; revision {page.revision}.
        </p>
      )}
      {page && page.revision !== acknowledgedRevision && (
        <div className="space-y-2">
          <p role="alert">
            The page changed since this form opened. Your fields remain here until you decide what
            to send.
          </p>
          <Button variant="outline" onClick={() => void refreshBaseline()}>
            Refresh Project and page
          </Button>
        </div>
      )}
      {(page === null || page?.status === "draft" || page?.status === "published") && (
        <div className="space-y-3">
          {(page === null || page?.status === "draft") && (
            <label className="block text-sm">
              Kind
              <select
                value={kind}
                onChange={(event) => setKind(event.target.value as typeof kind)}
                disabled={!editing}
                className="mt-1 w-full rounded border border-border bg-background p-2"
              >
                {kindOptions.map((option) => (
                  <option key={option} value={option}>
                    {option}
                  </option>
                ))}
              </select>
            </label>
          )}
          <label className="block text-sm">
            Title
            <Input
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              disabled={!editing}
            />
          </label>
          <label className="block text-sm">
            Markdown
            <textarea
              value={markdown}
              onChange={(event) => setMarkdown(event.target.value)}
              disabled={!editing}
              rows={8}
              className="mt-1 w-full rounded border border-border bg-background p-2 font-mono text-sm"
            />
          </label>
          <label className="block text-sm">
            Canonical source references (JSON array)
            <textarea
              value={sourcesText}
              onChange={(event) => setSourcesText(event.target.value)}
              disabled={!editing}
              rows={4}
              className="mt-1 w-full rounded border border-border bg-background p-2 font-mono text-sm"
            />
          </label>
          <p className="text-xs text-muted-foreground">
            Sources may cite Artifact, Event, Task handoff, or an exact Knowledge page revision.
            Empty [] is allowed for a human page.
          </p>
        </div>
      )}
      {fieldError && <p role="alert">{fieldError}</p>}
      {editing && !pendingAction && !baseline.isFetching && !baseline.isError && (
        <div className="flex flex-wrap gap-2">
          {(page === null || page?.status === "draft") && (
            <Button onClick={() => prepare("author_knowledge_page")}>
              {page ? "Save new draft revision" : "Create draft"}
            </Button>
          )}
          {page?.status === "draft" && (
            <Button variant="outline" onClick={() => setPendingAction("publish_knowledge_page")}>
              Publish
            </Button>
          )}
          {page?.status === "published" && (
            <Button onClick={() => prepare("supersede_knowledge_page")}>
              Supersede published page
            </Button>
          )}
          {(page?.status === "draft" || page?.status === "published") && (
            <Button
              variant="destructive"
              onClick={() => setPendingAction("withdraw_knowledge_page")}
            >
              Withdraw
            </Button>
          )}
        </div>
      )}
      {pendingAction && (
        <div
          role="alertdialog"
          aria-labelledby="knowledge-confirm-title"
          className="space-y-3 rounded border border-border p-4"
        >
          <h4 id="knowledge-confirm-title" ref={confirmRef} tabIndex={-1} className="font-medium">
            Confirm {pendingAction === "publish_knowledge_page" ? "publication" : "withdrawal"}
          </h4>
          <p className="text-sm">This creates a new canonical page revision.</p>
          <div className="flex gap-2">
            <Button onClick={() => prepare(pendingAction)}>Confirm</Button>
            <Button variant="outline" onClick={() => setPendingAction(null)}>
              Cancel
            </Button>
          </div>
        </div>
      )}
      {state.status === "sending" && <p role="status">Sending Knowledge command…</p>}
      {state.status === "unknown" && (
        <div className="space-y-2">
          <p role="alert">Outcome unknown. Retry the same request to obtain its receipt.</p>
          <Button disabled={inFlight.current} onClick={() => void submit(state.attempt)}>
            Retry same request
          </Button>
        </div>
      )}
      {state.status === "refused" && (
        <div className="space-y-2">
          <p role="alert">{refusal(state.error)}</p>
          <Button onClick={() => void refreshBaseline()}>Refresh Project and page</Button>
        </div>
      )}
      {state.status === "accepted" && (
        <div className="space-y-2">
          <p role="status">
            Command accepted. Receipt {state.receipt.command_id}.{" "}
            {state.readError ? "Canonical readback failed." : "Reading canonical page…"}
          </p>
          {state.readError && (
            <Button
              onClick={() => {
                if (lifetime.current) void readAccepted(state.receipt, lifetime.current);
              }}
            >
              Retry page read
            </Button>
          )}
        </div>
      )}
      {refreshError && <p role="alert">{refreshError}</p>}
      <Button
        variant="outline"
        onClick={() => {
          if (leaveGuard.canLeave()) onClose();
        }}
      >
        Close editor
      </Button>
    </section>
  );
}
