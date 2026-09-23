import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import { Input } from "../components/ui/input.tsx";
import { UuidV7Schema } from "../contracts/common.ts";
import {
  KnowledgePageSchema,
  type DerivedMemoryEntry,
  type KnowledgePage,
  type KnowledgeSource,
} from "../contracts/knowledge.ts";
import { describeApiError } from "./api.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";
import { KnowledgeCommandPanel } from "./knowledge-command-panel.tsx";

type Selection = { kind: "page" | "memory"; id: string };
type Navigation = { after: string | null; previous: (string | null)[] };
const initialNavigation: Navigation = { after: null, previous: [] };

export function KnowledgeBrowser(scope: ProjectReadScope) {
  const [view, setView] = useState<"pages" | "memory">("pages");
  return (
    <section aria-label="Knowledge and memory" className="space-y-5">
      <div className="space-y-3 rounded-xl border border-border bg-card p-6">
        <h2 className="text-lg font-semibold">Knowledge and memory</h2>
        <p className="text-sm text-muted-foreground">
          Canonical pages are managed by a human or Manager. Derived memory is generated from
          evidence and has no Policy or Decision authority. Canonical pages can be revised through
          explicit editorial commands; derived memory remains read only.
        </p>
        <div role="group" aria-label="Knowledge view" className="flex flex-wrap gap-2">
          <Button
            variant={view === "pages" ? "default" : "outline"}
            aria-pressed={view === "pages"}
            onClick={() => setView("pages")}
          >
            Canonical pages
          </Button>
          <Button
            variant={view === "memory" ? "default" : "outline"}
            aria-pressed={view === "memory"}
            onClick={() => setView("memory")}
          >
            Derived memory
          </Button>
        </div>
      </div>
      {view === "pages" ? <CanonicalPages {...scope} /> : <DerivedMemory {...scope} />}
    </section>
  );
}

function CanonicalPages(scope: ProjectReadScope) {
  const { api, session, generation, projectId } = scope;
  const queries = useQueryClient();
  const [navigation, setNavigation] = useState<Navigation>(initialNavigation);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [creating, setCreating] = useState(false);
  const key = useMemo(
    () => readKeys.knowledgePages(generation, projectId, navigation.after),
    [generation, projectId, navigation.after],
  );
  const pages = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.knowledgePages(projectId, navigation.after, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  function navigate(next: Navigation) {
    setSelection(null);
    setCreating(false);
    prepareReadChange(queries, key);
    setNavigation(next);
  }
  return (
    <>
      <section
        aria-label="Canonical pages"
        className="space-y-4 rounded-xl border border-border bg-card p-6"
      >
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h3 className="font-semibold">Canonical pages</h3>
          <div className="flex flex-wrap gap-2">
            <Button
              onClick={() => {
                setSelection(null);
                setCreating(true);
              }}
            >
              Create draft
            </Button>
            <Button
              variant="outline"
              disabled={pages.isFetching}
              onClick={() => void pages.refetch()}
            >
              Refresh pages
            </Button>
          </div>
        </div>
        <p className="text-xs text-muted-foreground">
          Includes drafts and withdrawn heads. Up to 20 pages per ID-ordered page; opening one reads
          its current canonical revision.
        </p>
        {pages.isPending && <p role="status">Loading canonical pages…</p>}
        {pages.isError && (
          <p role="alert">
            {pages.data ? "Showing stale pages. " : ""}
            {describeApiError(pages.error)}
          </p>
        )}
        {pages.data?.items.length === 0 && <p>No pages on this page.</p>}
        {pages.data && (
          <ul aria-label="Canonical page list" className="space-y-2">
            {pages.data.items.map((page) => (
              <li
                key={page.id}
                className="rounded border border-border p-3 text-sm [overflow-wrap:anywhere]"
              >
                <button
                  type="button"
                  aria-label={`Open canonical page ${page.id}`}
                  onClick={() => {
                    setCreating(false);
                    setSelection({ kind: "page", id: page.id });
                  }}
                  className="cursor-pointer rounded text-left font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  {page.content.title}
                </button>
                <p>
                  {page.kind} · {page.status} · revision {page.revision}
                </p>
              </li>
            ))}
          </ul>
        )}
        <Pagination
          label="Canonical page pagination"
          navigation={navigation}
          next={pages.isError ? null : (pages.data?.next_cursor ?? null)}
          busy={pages.isFetching}
          onNavigate={navigate}
        />
      </section>
      {creating && (
        <KnowledgeCommandPanel
          key="new-knowledge-page"
          {...scope}
          initialPage={null}
          onClose={() => setCreating(false)}
          onApplied={(pageId) => {
            setCreating(false);
            setSelection({ kind: "page", id: pageId });
          }}
        />
      )}
      {selection && (
        <KnowledgeDetail
          key={`${selection.kind}:${selection.id}`}
          {...scope}
          selection={selection}
          onClose={() => setSelection(null)}
        />
      )}
    </>
  );
}

function DerivedMemory(scope: ProjectReadScope) {
  const { api, session, generation, projectId } = scope;
  const queries = useQueryClient();
  const [employeeInput, setEmployeeInput] = useState("");
  const [employeeId, setEmployeeId] = useState<string | null>(null);
  const [scopeError, setScopeError] = useState(false);
  const [navigation, setNavigation] = useState<Navigation>(initialNavigation);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [searchInput, setSearchInput] = useState("");
  const [term, setTerm] = useState<string | null>(null);
  const [searchError, setSearchError] = useState(false);
  const key = useMemo(
    () => readKeys.memoryEntries(generation, projectId, employeeId, navigation.after),
    [generation, projectId, employeeId, navigation.after],
  );
  const entries = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.memoryEntries(projectId, employeeId, navigation.after, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  const statusKey = useMemo(
    () => readKeys.memoryStatus(generation, projectId),
    [generation, projectId],
  );
  const status = useQuery({
    queryKey: statusKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) => api.memoryStatus(projectId, token, signal)),
    retry: false,
  });
  useReadLifetime(statusKey);
  function navigate(next: Navigation) {
    setSelection(null);
    prepareReadChange(queries, key);
    setNavigation(next);
  }
  function changeEmployee(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const next = employeeInput.trim();
    if (next && !UuidV7Schema.safeParse(next).success) {
      setScopeError(true);
      return;
    }
    setScopeError(false);
    setSelection(null);
    setTerm(null);
    setNavigation(initialNavigation);
    setEmployeeId(next || null);
  }
  return (
    <>
      <section
        aria-label="Derived memory"
        className="space-y-4 rounded-xl border border-border bg-card p-6"
      >
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h3 className="font-semibold">Derived memory</h3>
          <Button
            variant="outline"
            disabled={entries.isFetching}
            onClick={() => void entries.refetch()}
          >
            Refresh memory
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          Project entries are shown by default. An Employee ID adds that Employee’s personal
          entries. Withdrawn entries and invalidated evidence are omitted from this list. The next
          cursor can exist even when fewer than 20 entries are visible.
        </p>
        <form onSubmit={changeEmployee} className="flex flex-wrap items-end gap-2">
          <label className="min-w-0 flex-1 text-sm">
            Employee ID (optional)
            <Input
              value={employeeInput}
              onChange={(event) => setEmployeeInput(event.target.value)}
              placeholder="UUIDv7 for personal entries"
            />
          </label>
          <Button type="submit" variant="outline">
            Apply scope
          </Button>
        </form>
        {scopeError && <p role="alert">Enter a valid Employee UUIDv7.</p>}
        <p className="text-xs text-muted-foreground">
          Current scope: {employeeId ? `Project and Employee ${employeeId}` : "Project only"}
        </p>
        {employeeId && <EmployeeOnboarding {...scope} employeeId={employeeId} />}
        {status.isPending && <p role="status">Loading projection status…</p>}
        {status.isError && (
          <p role="alert">Projection status unavailable. {describeApiError(status.error)}</p>
        )}
        {status.data && (
          <p className="text-xs text-muted-foreground">
            Search index: {status.data.configured ? "configured" : "not configured"}; indexed{" "}
            {status.data.indexed}, pending {status.data.pending}, retired {status.data.retired},
            failed attempts {status.data.failed_attempts}. Canonical maximum revision{" "}
            {status.data.canonical_max_revision}. Counts are diagnostics, not proof that every entry
            is searchable.
          </p>
        )}
        <form
          onSubmit={(event) => {
            event.preventDefault();
            const next = searchInput.trim();
            if (new TextEncoder().encode(next).length > 8192) {
              setSearchError(true);
              return;
            }
            setSearchError(false);
            setTerm(next || null);
            setSelection(null);
          }}
          className="flex flex-wrap items-end gap-2"
        >
          <label className="min-w-0 flex-1 text-sm">
            Search knowledge and memory
            <Input
              value={searchInput}
              onChange={(event) => setSearchInput(event.target.value)}
              maxLength={8192}
            />
          </label>
          <Button type="submit" variant="outline">
            Search
          </Button>
          <Button
            type="button"
            variant="outline"
            onClick={() => {
              setSearchInput("");
              setTerm(null);
            }}
          >
            Clear search
          </Button>
        </form>
        {searchError && <p role="alert">Search text must be at most 8192 UTF-8 bytes.</p>}
        {term && (
          <SearchResults
            key={`${employeeId}:${term}`}
            {...scope}
            employeeId={employeeId}
            term={term}
            onSelect={setSelection}
          />
        )}
        {entries.isPending && <p role="status">Loading derived memory…</p>}
        {entries.isError && (
          <p role="alert">
            {entries.data ? "Showing stale memory. " : ""}
            {describeApiError(entries.error)}
          </p>
        )}
        {entries.data?.items.length === 0 && <p>No visible entries on this page.</p>}
        {entries.data && (
          <ul aria-label="Derived memory list" className="space-y-2">
            {entries.data.items.map((entry) => (
              <li
                key={entry.id}
                className="rounded border border-border p-3 text-sm [overflow-wrap:anywhere]"
              >
                <button
                  type="button"
                  aria-label={`Open memory entry ${entry.id}`}
                  onClick={() => setSelection({ kind: "memory", id: entry.id })}
                  className="cursor-pointer rounded text-left font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  {entry.subject.kind} · revision {entry.revision}
                </button>
                <p>
                  {entry.subject.kind === "employee_memory_entry"
                    ? `Personal to ${entry.subject.employee_id}`
                    : "Project scope"}
                </p>
              </li>
            ))}
          </ul>
        )}
        <Pagination
          label="Derived memory pagination"
          navigation={navigation}
          next={entries.isError ? null : (entries.data?.next_cursor ?? null)}
          busy={entries.isFetching}
          onNavigate={navigate}
        />
      </section>
      {selection && (
        <KnowledgeDetail
          key={`${selection.kind}:${selection.id}`}
          {...scope}
          selection={selection}
          onClose={() => setSelection(null)}
        />
      )}
    </>
  );
}

function EmployeeOnboarding({ employeeId, ...scope }: ProjectReadScope & { employeeId: string }) {
  const { api, session, generation, projectId } = scope;
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
  return (
    <section
      aria-label="Employee onboarding"
      className="space-y-2 rounded border border-border p-3 text-sm"
    >
      <div className="flex items-center justify-between gap-2">
        <h4 className="font-medium">Employee onboarding</h4>
        <Button
          variant="outline"
          disabled={status.isFetching}
          onClick={() => void status.refetch()}
        >
          Refresh onboarding
        </Button>
      </div>
      {status.isPending && <p role="status">Loading onboarding…</p>}
      {status.isError && (
        <p role="alert">
          {status.data ? "Showing stale onboarding. " : ""}
          {describeApiError(status.error, "Employee")}
        </p>
      )}
      {status.data && (
        <p>
          State: {status.data.state}; revision {status.data.revision}; job{" "}
          {status.data.job_id ?? "none"}. A job receipt is{" "}
          {status.data.receipt === null ? "not recorded" : "recorded"}.
        </p>
      )}
    </section>
  );
}

function SearchResults({
  employeeId,
  term,
  onSelect,
  ...scope
}: ProjectReadScope & {
  employeeId: string | null;
  term: string;
  onSelect: (selection: Selection) => void;
}) {
  const { api, session, generation, projectId } = scope;
  const key = useMemo(
    () => readKeys.memorySearch(generation, projectId, employeeId, term),
    [generation, projectId, employeeId, term],
  );
  const result = useQuery({
    queryKey: key,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.memorySearch(projectId, employeeId, term, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(key);
  return (
    <section
      aria-label="Search results"
      className="space-y-2 rounded border border-border p-3 text-sm"
    >
      <div className="flex items-center justify-between gap-2">
        <h4 className="font-medium">Search results</h4>
        <Button
          variant="outline"
          disabled={result.isFetching}
          onClick={() => void result.refetch()}
        >
          Refresh search
        </Button>
      </div>
      {result.isPending && <p role="status">Searching…</p>}
      {result.isError && (
        <p role="alert">
          {result.data ? "Showing stale search results. " : ""}
          {describeApiError(result.error)}
        </p>
      )}
      {result.data && (
        <>
          <p>
            Mode: {result.data.mode}.{" "}
            {result.data.degradation ? `Degraded: ${result.data.degradation}.` : ""}{" "}
            {result.data.truncated
              ? "Results truncated; search is not exhaustive."
              : "Search is bounded, not exhaustive."}
          </p>
          <ul className="space-y-1">
            {result.data.results.map((hit, index) => (
              <li key={`${hit.document.kind}:${hit.document.record.id}:${index}`}>
                <button
                  type="button"
                  className="cursor-pointer rounded text-left text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() =>
                    onSelect({
                      kind: hit.document.kind === "knowledge_page" ? "page" : "memory",
                      id: hit.document.record.id,
                    })
                  }
                >
                  {hit.document.kind === "knowledge_page"
                    ? hit.document.record.content.title
                    : hit.document.record.subject.kind}{" "}
                  · revision {hit.document.record.revision}
                </button>
              </li>
            ))}
          </ul>
          {result.data.results.length === 0 && <p>No results in this bounded search.</p>}
        </>
      )}
    </section>
  );
}

function KnowledgeDetail({
  selection,
  onClose,
  ...scope
}: ProjectReadScope & { selection: Selection; onClose: () => void }) {
  const { api, session, generation, projectId } = scope;
  const [navigation, setNavigation] = useState<Navigation>(initialNavigation);
  const [editing, setEditing] = useState(false);
  const [selectedRevision, setSelectedRevision] = useState<number | null>(null);
  const detailKey = useMemo(
    () =>
      selection.kind === "page"
        ? readKeys.knowledgePage(generation, projectId, selection.id)
        : readKeys.memoryEntry(generation, projectId, selection.id),
    [selection, generation, projectId],
  );
  const detail = useQuery<KnowledgePage | DerivedMemoryEntry>({
    queryKey: detailKey,
    queryFn: ({ signal }) =>
      session.request<KnowledgePage | DerivedMemoryEntry>(generation, (token) =>
        selection.kind === "page"
          ? api.knowledgePage(projectId, selection.id, token, signal)
          : api.memoryEntry(projectId, selection.id, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(detailKey);
  const historyKey = useMemo(
    () =>
      selection.kind === "page"
        ? readKeys.knowledgeHistory(generation, projectId, selection.id, navigation.after)
        : readKeys.memoryHistory(generation, projectId, selection.id, navigation.after),
    [selection, generation, projectId, navigation.after],
  );
  const history = useQuery<{
    items: (KnowledgePage | DerivedMemoryEntry)[];
    next_cursor: string | null;
  }>({
    queryKey: historyKey,
    queryFn: ({ signal }) =>
      session.request<{
        items: (KnowledgePage | DerivedMemoryEntry)[];
        next_cursor: string | null;
      }>(generation, (token) =>
        selection.kind === "page"
          ? api.knowledgeHistory(projectId, selection.id, navigation.after, token, signal)
          : api.memoryHistory(projectId, selection.id, navigation.after, token, signal),
      ),
    retry: false,
  });
  useReadLifetime(historyKey);
  const record = detail.data;
  const historicalRecord = history.data?.items.find((item) => item.revision === selectedRevision);
  return (
    <section
      aria-label="Knowledge record details"
      className="space-y-4 rounded-xl border border-border bg-card p-6"
    >
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="font-semibold">
          {selection.kind === "page" ? "Canonical page" : "Derived memory"} details
        </h3>
        <div className="flex gap-2">
          <Button
            variant="outline"
            disabled={detail.isFetching}
            onClick={() => void detail.refetch()}
          >
            Refresh record
          </Button>
          <Button variant="outline" onClick={onClose}>
            Close details
          </Button>
        </div>
      </div>
      {detail.isPending && <p role="status">Loading record…</p>}
      {detail.isError && (
        <p role="alert">
          {record ? "Showing stale record. " : ""}
          {describeApiError(detail.error)}
        </p>
      )}
      {record && <RecordFacts record={record} />}
      {record &&
        selection.kind === "page" &&
        isKnowledgePage(record) &&
        !editing &&
        record.status !== "withdrawn" && (
          <Button variant="outline" onClick={() => setEditing(true)}>
            Manage page revisions
          </Button>
        )}
      {record && selection.kind === "page" && isKnowledgePage(record) && editing && (
        <KnowledgeCommandPanel
          key={record.id}
          {...scope}
          initialPage={record}
          onClose={() => setEditing(false)}
          onApplied={() => {
            setEditing(false);
            void detail.refetch();
            void history.refetch();
          }}
        />
      )}
      <section aria-label="Revision history" className="space-y-2">
        <h4 className="font-medium">Revision history</h4>
        {history.isPending && <p role="status">Loading history…</p>}
        {history.isError && (
          <p role="alert">
            {history.data ? "Showing stale history. " : ""}
            {describeApiError(history.error)}
          </p>
        )}
        {history.data && (
          <ul className="space-y-2">
            {history.data.items.map((item) => (
              <li key={item.revision} className="rounded border border-border p-2 text-sm">
                <button
                  type="button"
                  aria-label={`Inspect revision ${item.revision}`}
                  aria-pressed={selectedRevision === item.revision}
                  onClick={() => setSelectedRevision(item.revision)}
                  className="cursor-pointer rounded text-left text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  Revision {item.revision} ·{" "}
                  {isKnowledgePage(item) ? item.status : item.withdrawn ? "withdrawn" : "active"} ·{" "}
                  {item.created_at}
                </button>
              </li>
            ))}
          </ul>
        )}
        {historicalRecord && (
          <section
            aria-label={`Revision ${historicalRecord.revision} contents`}
            className="rounded border border-border p-3"
          >
            <h5 className="font-medium">Revision {historicalRecord.revision} snapshot</h5>
            <RecordFacts record={historicalRecord} />
          </section>
        )}
        <Pagination
          label="Revision history pagination"
          navigation={navigation}
          next={history.isError ? null : (history.data?.next_cursor ?? null)}
          busy={history.isFetching}
          onNavigate={(next) => {
            setSelectedRevision(null);
            setNavigation(next);
          }}
        />
      </section>
    </section>
  );
}

function RecordFacts({ record }: { record: KnowledgePage | DerivedMemoryEntry }) {
  const page = isKnowledgePage(record);
  const sources = page ? record.content.source_refs : record.source_refs;
  return (
    <div className="space-y-3 text-sm [overflow-wrap:anywhere]">
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1">
        <dt>ID</dt>
        <dd className="font-mono">{record.id}</dd>
        <dt>Revision</dt>
        <dd>{record.revision}</dd>
        <dt>Content hash</dt>
        <dd className="font-mono">{record.content_hash}</dd>
        {page ? (
          <>
            <dt>Kind / status</dt>
            <dd>
              {record.kind} / {record.status}
            </dd>
            <dt>Author</dt>
            <dd>
              {record.actor.kind} · {record.actor.id}
            </dd>
            <dt>Command</dt>
            <dd>{record.command_id}</dd>
            <dt>Revised</dt>
            <dd>{record.revised_at}</dd>
          </>
        ) : (
          <>
            <dt>Subject</dt>
            <dd>
              {record.subject.kind}
              {record.subject.kind === "employee_memory_entry"
                ? ` · ${record.subject.employee_id}`
                : ""}
            </dd>
            <dt>Generated by job</dt>
            <dd>{record.created_by_job_id}</dd>
            <dt>Withdrawn</dt>
            <dd>{record.withdrawn ? "Yes" : "No"}</dd>
            <dt>Event coverage</dt>
            <dd>
              {record.coverage
                ? `${record.coverage.first_event_sequence}–${record.coverage.last_event_sequence}`
                : "Not recorded"}
            </dd>
          </>
        )}
      </dl>
      <div>
        <h4 className="font-medium">{page ? record.content.title : "Generated content"}</h4>
        <p className="whitespace-pre-wrap">{page ? record.content.markdown : record.markdown}</p>
      </div>
      <div>
        <h4 className="font-medium">Canonical sources</h4>
        {sources.length ? (
          <ul className="list-inside list-disc">
            {sources.map((source, index) => (
              <li key={`${source.kind}:${index}`}>
                <SourceRef source={source} />
              </li>
            ))}
          </ul>
        ) : (
          <p>No source references recorded.</p>
        )}
      </div>
    </div>
  );
}

function isKnowledgePage(record: KnowledgePage | DerivedMemoryEntry): record is KnowledgePage {
  return KnowledgePageSchema.safeParse(record).success;
}

function SourceRef({ source }: { source: KnowledgeSource }) {
  switch (source.kind) {
    case "artifact":
      return <>Artifact {source.artifact_id}</>;
    case "event":
      return <>Event {source.event_id}</>;
    case "task_handoff":
      return <>Task handoff {source.handoff_id}</>;
    case "knowledge_page":
      return (
        <>
          Knowledge page {source.page_id}, revision {source.revision}
        </>
      );
  }
}

function Pagination({
  label,
  navigation,
  next,
  busy,
  onNavigate,
}: {
  label: string;
  navigation: Navigation;
  next: string | null;
  busy: boolean;
  onNavigate: (next: Navigation) => void;
}) {
  return (
    <nav aria-label={label} className="flex flex-wrap items-center justify-between gap-3 text-sm">
      <Button
        variant="outline"
        disabled={busy || navigation.previous.length === 0}
        onClick={() =>
          onNavigate({
            after: navigation.previous.at(-1) ?? null,
            previous: navigation.previous.slice(0, -1),
          })
        }
      >
        Previous page
      </Button>
      <span>Page {navigation.previous.length + 1}</span>
      <Button
        variant="outline"
        disabled={busy || next === null}
        onClick={() => {
          if (next !== null)
            onNavigate({ after: next, previous: [...navigation.previous, navigation.after] });
        }}
      >
        Next page
      </Button>
    </nav>
  );
}
