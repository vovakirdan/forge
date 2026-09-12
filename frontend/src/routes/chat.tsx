import { createFileRoute, Link } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { ChatService, AgentService } from "@/services";
import type { ChatCard, ChatMessage } from "@/data/types";
import { Page, PageHeader, PageBody } from "@/components/common/Page";
import { Chip, Dot, Initials, Panel } from "@/components/common/Bits";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { clockTime } from "@/lib/format";
import { toast } from "sonner";
import {
  AlertTriangle,
  CheckCircle2,
  GitPullRequestArrow,
  HelpCircle,
  Search,
  Send,
  XCircle,
} from "lucide-react";

export const Route = createFileRoute("/chat")({
  head: () => ({
    meta: [
      { title: "Chat with the Lead — Forge" },
      {
        name: "description",
        content:
          "Talk directly to the Lead agent about live project status, decisions and reviews without opening a ticket.",
      },
      { property: "og:title", content: "Chat with the Lead — Forge" },
      {
        property: "og:description",
        content: "Structured, system-connected conversation — not a Slack clone.",
      },
    ],
  }),
  component: ChatPage,
});

const cardMeta: Record<
  ChatCard["kind"],
  {
    label: string;
    tone: "primary" | "warning" | "success" | "danger" | "info";
    icon: React.ReactNode;
  }
> = {
  decision: {
    label: "Decision required",
    tone: "primary",
    icon: <HelpCircle className="size-3.5" />,
  },
  review_request: {
    label: "Review requested",
    tone: "warning",
    icon: <GitPullRequestArrow className="size-3.5" />,
  },
  task_completed: {
    label: "Task completed",
    tone: "success",
    icon: <CheckCircle2 className="size-3.5" />,
  },
  verification_failed: {
    label: "Verification failed",
    tone: "danger",
    icon: <XCircle className="size-3.5" />,
  },
  risk: { label: "Risk detected", tone: "danger", icon: <AlertTriangle className="size-3.5" /> },
  finding: { label: "Finding reported", tone: "info", icon: <Search className="size-3.5" /> },
};

function ChatPage() {
  const qc = useQueryClient();
  const [channelId, setChannelId] = useState("ch-lead");
  const [draft, setDraft] = useState("");
  const [pending, setPending] = useState<ChatMessage[]>([]);
  const [thinking, setThinking] = useState(false);
  const endRef = useRef<HTMLDivElement>(null);

  const { data: channels } = useQuery({ queryKey: ["channels"], queryFn: ChatService.channels });
  const { data: agents } = useQuery({ queryKey: ["agents"], queryFn: AgentService.list });
  const { data: messages } = useQuery({
    queryKey: ["messages", channelId],
    queryFn: () => ChatService.messages(channelId),
  });

  useEffect(() => setPending([]), [channelId]);
  useEffect(() => {
    endRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, pending, thinking]);

  const all = [...(messages ?? []), ...pending];
  const channel = channels?.find((c) => c.id === channelId);

  const send = async () => {
    const body = draft.trim();
    if (!body) return;
    setDraft("");
    const mine = await ChatService.send(channelId, body);
    setPending((p) => [...p, mine]);
    setThinking(true);
    const reply = await ChatService.leadReply(channelId, body);
    setThinking(false);
    setPending((p) => [...p, reply]);
    qc.invalidateQueries({ queryKey: ["activity"] });
  };

  return (
    <Page>
      <PageHeader
        title="Chat"
        subtitle="Direct line to the Lead. Talking here never creates a task by itself."
      />
      <div className="grid min-h-0 flex-1 grid-cols-[240px_1fr] divide-x divide-border">
        <aside className="min-h-0 overflow-auto bg-surface">
          {!channels ? (
            <div className="space-y-2 p-3">
              {Array.from({ length: 6 }).map((_, i) => (
                <Skeleton key={i} className="h-9 w-full" />
              ))}
            </div>
          ) : (
            ["agent", "project", "system"].map((kind) => (
              <div key={kind}>
                <p className="section-label px-3 pt-3 pb-1">{kind}</p>
                <ul>
                  {channels
                    .filter((c) => c.kind === kind)
                    .map((c) => {
                      const agent = agents?.find((a) => a.id === c.participantId);
                      return (
                        <li key={c.id}>
                          <button
                            onClick={() => setChannelId(c.id)}
                            className={cn(
                              "flex w-full items-center gap-2 px-3 py-1.5 text-left transition-colors hover:bg-accent/50",
                              c.id === channelId && "bg-accent",
                            )}
                          >
                            {agent ? (
                              <Initials id={agent.id} name={agent.name} size={20} />
                            ) : (
                              <span className="inline-flex size-5 items-center justify-center rounded border border-border bg-muted text-[9px] text-muted-foreground">
                                #
                              </span>
                            )}
                            <span className="min-w-0 flex-1">
                              <span className="block truncate text-[12.5px] font-medium">
                                {c.name}
                              </span>
                              <span className="block truncate text-[11px] text-muted-foreground">
                                {c.subtitle}
                              </span>
                            </span>
                            {c.unread > 0 && (
                              <span className="rounded bg-primary px-1 text-[10px] font-semibold text-primary-foreground">
                                {c.unread}
                              </span>
                            )}
                          </button>
                        </li>
                      );
                    })}
                </ul>
              </div>
            ))
          )}
        </aside>

        <section className="flex min-h-0 flex-col">
          <div className="flex shrink-0 items-center gap-2 border-b border-border px-4 py-2">
            <span className="text-[13px] font-semibold">{channel?.name ?? "Conversation"}</span>
            <span className="text-[11.5px] text-muted-foreground">{channel?.subtitle}</span>
            <Chip tone="info" className="ml-auto">
              structured project status attached
            </Chip>
          </div>

          <div className="min-h-0 flex-1 overflow-auto px-4 py-3">
            {!messages ? (
              <div className="space-y-3">
                {Array.from({ length: 5 }).map((_, i) => (
                  <Skeleton key={i} className="h-16 w-2/3" />
                ))}
              </div>
            ) : (
              <ul className="space-y-4">
                {all.map((m) => {
                  const author = agents?.find((a) => a.id === m.authorId);
                  const isHuman = m.authorId === "human";
                  return (
                    <li key={m.id} className="flex gap-2.5">
                      <Initials
                        id={isHuman ? "human" : (author?.id ?? "system")}
                        name={isHuman ? "You" : (author?.name ?? "System")}
                        size={22}
                      />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-baseline gap-2">
                          <span className="text-[12.5px] font-semibold">
                            {isHuman ? "You" : (author?.name ?? "System")}
                          </span>
                          {author && (
                            <span className="text-[11px] text-muted-foreground">{author.role}</span>
                          )}
                          <span className="mono-xs text-muted-foreground">
                            {clockTime(m.createdAt)}
                          </span>
                        </div>
                        {m.body && (
                          <p className="mt-0.5 max-w-3xl text-[12.5px] leading-relaxed whitespace-pre-line">
                            {m.body}
                          </p>
                        )}
                        {m.card && <StructuredCard card={m.card} />}
                      </div>
                    </li>
                  );
                })}
                {thinking && (
                  <li className="flex items-center gap-2 text-[12px] text-muted-foreground">
                    <Dot tone="running" pulse /> Lead is aggregating project status…
                  </li>
                )}
              </ul>
            )}
            <div ref={endRef} />
          </div>

          <div className="shrink-0 border-t border-border p-3">
            <div className="rounded-md border border-border bg-surface focus-within:border-border-strong">
              <Textarea
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    void send();
                  }
                }}
                placeholder="Ask the Lead what is happening right now…"
                className="min-h-16 resize-none border-0 bg-transparent text-[12.5px] shadow-none focus-visible:ring-0"
              />
              <div className="flex items-center justify-between px-2 pb-2">
                <div className="flex gap-1">
                  {[
                    "What is happening right now?",
                    "Anything blocked?",
                    "Summarise today's failures",
                  ].map((q) => (
                    <Button
                      key={q}
                      size="sm"
                      variant="ghost"
                      className="h-6 text-[11px] text-muted-foreground"
                      onClick={() => setDraft(q)}
                    >
                      {q}
                    </Button>
                  ))}
                </div>
                <Button
                  size="sm"
                  className="h-7 text-[12px]"
                  onClick={() => void send()}
                  disabled={!draft.trim()}
                >
                  <Send className="size-3.5" /> Send
                </Button>
              </div>
            </div>
          </div>
        </section>
      </div>
    </Page>
  );
}

function StructuredCard({ card }: { card: ChatCard }) {
  const meta = cardMeta[card.kind];
  return (
    <div className="mt-2 max-w-2xl overflow-hidden rounded-md border border-border bg-surface-raised">
      <div className="flex items-center gap-2 border-b border-border px-2.5 py-1.5">
        <span className="text-muted-foreground">{meta.icon}</span>
        <span className="section-label">{meta.label}</span>
        {card.taskId && (
          <Link
            to="/tasks/$taskId"
            params={{ taskId: card.taskId }}
            className="mono-xs ml-auto text-primary hover:underline"
          >
            {card.taskId}
          </Link>
        )}
      </div>
      <div className="px-2.5 py-2">
        <p className="text-[12.5px] font-medium">{card.title}</p>
        <p className="mt-0.5 text-[12px] leading-relaxed text-muted-foreground">{card.body}</p>
        {card.options && (
          <ul className="mt-2 space-y-1.5">
            {card.options.map((o) => (
              <li key={o.id} className="rounded border border-border bg-surface px-2 py-1.5">
                <span className="text-[12px] font-medium">{o.label}</span>
                <p className="text-[11.5px] text-muted-foreground">{o.detail}</p>
              </li>
            ))}
          </ul>
        )}
        {card.actions && (
          <div className="mt-2 flex flex-wrap gap-1.5">
            {card.actions.map((a, i) => (
              <Button
                key={a}
                size="sm"
                variant={i === 0 ? "default" : "outline"}
                className="h-6 text-[11.5px]"
                onClick={() =>
                  toast.success(`${a} recorded`, {
                    description: card.taskId ? `${card.taskId} updated` : undefined,
                  })
                }
              >
                {a}
              </Button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
