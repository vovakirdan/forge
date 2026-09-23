import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "../components/ui/button.tsx";
import type {
  EmployeeMessage,
  EmployeeThread,
  MessageDelivery,
} from "../contracts/communication.ts";
import { InboxActions } from "./InboxActions.tsx";
import { CommunicationRetry } from "./CommunicationRetry.tsx";
import { describeApiError } from "./api.ts";
import { prepareReadChange, readKeys } from "./read-cache.ts";
import type { ProjectReadScope } from "./read-scope.ts";
import { useReadLifetime } from "./use-read-lifetime.ts";

type Page = { after: string | null; previous: (string | null)[] };
const FIRST_PAGE: Page = { after: null, previous: [] };

export function EmployeeInbox({
  scope,
  employeeId,
}: {
  scope: ProjectReadScope;
  employeeId: string;
}) {
  const { api, session, generation, projectId } = scope;
  const queries = useQueryClient();
  const [threadPage, setThreadPage] = useState<Page>(FIRST_PAGE);
  const [messagePage, setMessagePage] = useState<Page>(FIRST_PAGE);
  const [threadId, setThreadId] = useState<string | null>(null);
  const threadKey = useMemo(
    () => readKeys.employeeThreads(generation, projectId, employeeId, threadPage.after),
    [generation, projectId, employeeId, threadPage.after],
  );
  const messageKey = useMemo(
    () =>
      readKeys.employeeMessages(
        generation,
        projectId,
        employeeId,
        threadId ?? "",
        messagePage.after,
      ),
    [generation, projectId, employeeId, threadId, messagePage.after],
  );
  const threads = useQuery({
    queryKey: threadKey,
    queryFn: ({ signal }) =>
      session.request(generation, (token) =>
        api.employeeThreads(projectId, employeeId, threadPage.after, token, signal),
      ),
    retry: false,
  });
  const messages = useQuery({
    queryKey: messageKey,
    queryFn: ({ signal }) => {
      if (threadId === null) throw new Error("No thread selected");
      return session.request(generation, (token) =>
        api.employeeMessages(projectId, employeeId, threadId, messagePage.after, token, signal),
      );
    },
    enabled: threadId !== null,
    retry: false,
  });
  const deliveryKey = useMemo(
    () => readKeys.messageDelivery(generation, projectId, threadId ?? "", messagePage.after),
    [generation, projectId, threadId, messagePage.after],
  );
  const delivery = useQuery({
    queryKey: deliveryKey,
    queryFn: ({ signal }) => {
      if (threadId === null) throw new Error("No thread selected");
      return session.request(generation, (token) =>
        api.messageDelivery(projectId, threadId, messagePage.after, token, signal),
      );
    },
    enabled: threadId !== null,
    retry: false,
  });
  const selectedThread = threads.data?.items.find((thread) => thread.id === threadId) ?? null;
  useReadLifetime(threadKey);
  useReadLifetime(messageKey);
  useReadLifetime(deliveryKey);

  function selectThread(id: string | null) {
    if (threadId === id) return;
    prepareReadChange(queries, messageKey);
    prepareReadChange(queries, deliveryKey);
    setThreadId(id);
    setMessagePage(FIRST_PAGE);
  }
  function navigateThreads(next: Page) {
    selectThread(null);
    prepareReadChange(queries, threadKey);
    setThreadPage(next);
  }
  function navigateMessages(next: Page) {
    prepareReadChange(queries, messageKey);
    prepareReadChange(queries, deliveryKey);
    setMessagePage(next);
  }

  return (
    <section aria-label="Employee Inbox" className="space-y-4 border-t border-border pt-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h3 className="font-medium">Inbox</h3>
        <Button
          variant="outline"
          disabled={threads.isFetching}
          onClick={() => void threads.refetch()}
        >
          Refresh threads
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Saved conversations, up to 20 per page. A saved message does not prove delivery,
        acknowledgement, or an answer. Reading does not acknowledge it.
      </p>
      <InboxActions
        scope={scope}
        employeeId={employeeId}
        thread={null}
        onApplied={() => {
          selectThread(null);
          void threads.refetch();
        }}
      />
      {threads.isPending && <p role="status">Loading threads…</p>}
      {threads.isError && (
        <p role="alert">
          {threads.data ? "Showing stale threads. " : ""}
          {describeApiError(threads.error, "Employee")}
        </p>
      )}
      {threads.data?.items.length === 0 && <p>No threads on this page.</p>}
      {threads.data && threads.data.items.length > 0 && (
        <ul aria-label="Employee threads" className="space-y-2">
          {threads.data.items.map((thread) => (
            <li key={thread.id} className="rounded border border-border p-3">
              <button
                type="button"
                aria-label={`Open thread ${thread.id}`}
                aria-expanded={threadId === thread.id}
                onClick={() => selectThread(thread.id)}
                className="cursor-pointer rounded text-left text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [overflow-wrap:anywhere]"
              >
                Thread {thread.id}
              </button>
              <p className="text-xs text-muted-foreground [overflow-wrap:anywhere]">
                {thread.task_id ? `Task ${thread.task_id}` : "No Task linked"} · Updated{" "}
                {thread.updated_at} · Last sequence {thread.last_sequence}
              </p>
            </li>
          ))}
        </ul>
      )}
      <nav
        aria-label="Thread pagination"
        className="flex flex-wrap items-center justify-between gap-3"
      >
        <Button
          variant="outline"
          disabled={threads.isFetching || threadPage.previous.length === 0}
          onClick={() =>
            navigateThreads({
              after: threadPage.previous.at(-1) ?? null,
              previous: threadPage.previous.slice(0, -1),
            })
          }
        >
          Previous thread page
        </Button>
        <p className="text-sm">Page {threadPage.previous.length + 1}</p>
        <Button
          variant="outline"
          disabled={threads.isFetching || threads.isError || !threads.data?.next_cursor}
          onClick={() => {
            const after = threads.data?.next_cursor;
            if (after)
              navigateThreads({ after, previous: [...threadPage.previous, threadPage.after] });
          }}
        >
          Next thread page
        </Button>
      </nav>
      {threadId && (
        <section aria-label="Thread messages" className="space-y-3 border-t border-border pt-4">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <h4 className="font-medium [overflow-wrap:anywhere]">Messages in {threadId}</h4>
            <Button
              variant="outline"
              disabled={messages.isFetching}
              onClick={() => {
                void messages.refetch();
                void delivery.refetch();
              }}
            >
              Refresh messages
            </Button>
          </div>
          {selectedThread && (
            <InboxActions
              key={selectedThread.id}
              scope={scope}
              employeeId={employeeId}
              thread={selectedThread}
              onApplied={() => {
                void threads.refetch();
                void messages.refetch();
                void delivery.refetch();
              }}
            />
          )}
          {delivery.isError && (
            <p role="alert">
              Delivery evidence unavailable. {describeApiError(delivery.error, "Employee")}
            </p>
          )}
          {delivery.isPending && <p role="status">Loading delivery evidence…</p>}
          {messages.isPending && <p role="status">Loading messages…</p>}
          {messages.isError && (
            <p role="alert">
              {messages.data ? "Showing stale messages. " : ""}
              {describeApiError(messages.error, "Employee")}
            </p>
          )}
          {messages.data?.items.length === 0 && <p>No messages on this page.</p>}
          {messages.data && messages.data.items.length > 0 && (
            <ol aria-label="Saved messages" className="space-y-3">
              {messages.data.items.map((message) => (
                <EmployeeMessageCard
                  key={message.id}
                  message={message}
                  delivery={delivery.data?.items.find((item) => item.message_id === message.id)}
                  scope={scope}
                  employeeId={employeeId}
                  thread={selectedThread}
                  onApplied={() => {
                    void delivery.refetch();
                    void messages.refetch();
                  }}
                />
              ))}
            </ol>
          )}
          <nav
            aria-label="Message pagination"
            className="flex flex-wrap items-center justify-between gap-3"
          >
            <Button
              variant="outline"
              disabled={messages.isFetching || messagePage.previous.length === 0}
              onClick={() =>
                navigateMessages({
                  after: messagePage.previous.at(-1) ?? null,
                  previous: messagePage.previous.slice(0, -1),
                })
              }
            >
              Previous message page
            </Button>
            <p className="text-sm">Page {messagePage.previous.length + 1}</p>
            <Button
              variant="outline"
              disabled={messages.isFetching || messages.isError || !messages.data?.next_cursor}
              onClick={() => {
                const after = messages.data?.next_cursor;
                if (after)
                  navigateMessages({
                    after,
                    previous: [...messagePage.previous, messagePage.after],
                  });
              }}
            >
              Next message page
            </Button>
          </nav>
        </section>
      )}
    </section>
  );
}

export function EmployeeMessageCard({
  message,
  delivery,
  scope,
  employeeId,
  thread,
  onApplied,
}: {
  message: EmployeeMessage;
  delivery?: MessageDelivery | undefined;
  scope?: ProjectReadScope;
  employeeId?: string;
  thread?: EmployeeThread | null;
  onApplied?: () => void;
}) {
  return (
    <li className="rounded border border-border p-3">
      <p className="text-xs text-muted-foreground [overflow-wrap:anywhere]">
        Sequence {message.sequence} · {message.kind} · {message.created_at} · Sender{" "}
        {message.sender.kind} ({message.sender.id})
      </p>
      <p className="whitespace-pre-wrap text-sm [overflow-wrap:anywhere]">{message.body}</p>
      <p className="text-xs text-muted-foreground">
        Target: {message.target.kind} · Requested handling: {message.requirement}
      </p>
      {delivery ? (
        <p className="text-xs text-muted-foreground [overflow-wrap:anywhere]">
          {delivery.assignment
            ? `Assignment ${delivery.assignment.state}, attempt ${delivery.assignment.attempt_number}${delivery.assignment.run_id ? `, Run ${delivery.assignment.run_id}` : ""}`
            : "No Inbox assignment"}
          {delivery.runtime_accepted_at
            ? ` · Runtime accepted ${delivery.runtime_accepted_at}`
            : " · No runtime acceptance receipt"}
          {delivery.acknowledged_at
            ? ` · Acknowledged ${delivery.acknowledged_at}`
            : " · No acknowledgement receipt"}
          {delivery.answered_at
            ? ` · Answered ${delivery.answered_at}, reply ${delivery.answered_reply_id}`
            : " · No answer receipt"}
          {delivery.waiver
            ? ` · Waived by management ${delivery.waiver.created_at}: ${delivery.waiver.reason}`
            : ""}
        </p>
      ) : (
        <p className="text-xs text-muted-foreground">
          Delivery evidence not loaded for this message.
        </p>
      )}
      {scope && employeeId && thread && onApplied && (
        <InboxActions
          scope={scope}
          employeeId={employeeId}
          thread={thread}
          message={message}
          delivery={delivery}
          onApplied={onApplied}
        />
      )}
      {scope && thread && delivery && onApplied && (
        <CommunicationRetry
          scope={scope}
          threadId={thread.id}
          delivery={delivery}
          onApplied={onApplied}
        />
      )}
    </li>
  );
}
