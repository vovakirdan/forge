import { cn } from "@/lib/utils";
import { initials } from "@/lib/format";
import type { ReactNode } from "react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";

/* ---------------- status dot ---------------- */

const toneClass = {
  success: "bg-success",
  danger: "bg-destructive",
  warning: "bg-warning",
  info: "bg-info",
  running: "bg-running",
  muted: "bg-muted-foreground",
  primary: "bg-primary",
} as const;

export type Tone = keyof typeof toneClass;

export function Dot({ tone = "muted", pulse }: { tone?: Tone | undefined; pulse?: boolean }) {
  return (
    <span
      className={cn(
        "inline-block size-1.5 shrink-0 rounded-full",
        toneClass[tone],
        pulse && "live-dot",
      )}
    />
  );
}

/* ---------------- chip / badge ---------------- */

const chipTone: Record<Tone, string> = {
  success: "border-success/30 bg-success/10 text-success",
  danger: "border-destructive/30 bg-destructive/10 text-destructive",
  warning: "border-warning/35 bg-warning/12 text-warning",
  info: "border-info/30 bg-info/10 text-info",
  running: "border-running/30 bg-running/10 text-running",
  muted: "border-border bg-muted text-muted-foreground",
  primary: "border-primary/30 bg-primary/10 text-primary",
};

export function Chip({
  tone = "muted",
  children,
  className,
  icon,
  mono,
}: {
  tone?: Tone | undefined;
  children: ReactNode;
  className?: string | undefined;
  icon?: ReactNode | undefined;
  mono?: boolean | undefined;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded border px-1.5 py-px text-[10.5px] font-medium leading-4 whitespace-nowrap",
        chipTone[tone],
        mono && "font-mono",
        className,
      )}
    >
      {icon}
      {children}
    </span>
  );
}

/* ---------------- avatar ---------------- */

const avatarTones: Record<string, string> = {
  "agt-max": "bg-chart-1/20 text-chart-1 border-chart-1/30",
  "agt-bob": "bg-chart-2/20 text-chart-2 border-chart-2/30",
  "agt-alice": "bg-chart-3/20 text-chart-3 border-chart-3/30",
  "agt-john": "bg-chart-5/20 text-chart-5 border-chart-5/30",
  human: "bg-primary/20 text-primary border-primary/30",
  system: "bg-muted text-muted-foreground border-border",
};

export function Initials({
  id,
  name,
  size = 22,
  className,
}: {
  id?: string | undefined;
  name: string;
  size?: number | undefined;
  className?: string | undefined;
}) {
  return (
    <span
      style={{ width: size, height: size, fontSize: Math.round(size * 0.4) }}
      className={cn(
        "inline-flex shrink-0 items-center justify-center rounded border font-semibold",
        avatarTones[id ?? ""] ?? "bg-secondary text-secondary-foreground border-border",
        className,
      )}
    >
      {initials(name)}
    </span>
  );
}

/* ---------------- meter ---------------- */

export function Meter({
  value,
  tone = "primary",
  className,
}: {
  value: number;
  tone?: Tone | undefined;
  className?: string | undefined;
}) {
  return (
    <div className={cn("h-1.5 w-full overflow-hidden rounded-full bg-muted", className)}>
      <div
        className={cn("h-full rounded-full transition-all", toneClass[tone])}
        style={{ width: `${Math.min(100, Math.max(0, value))}%` }}
      />
    </div>
  );
}

/* ---------------- layout helpers ---------------- */

export function Panel({
  title,
  action,
  children,
  className,
  bodyClassName,
  dense,
}: {
  title?: ReactNode | undefined;
  action?: ReactNode | undefined;
  children: ReactNode;
  className?: string | undefined;
  bodyClassName?: string | undefined;
  dense?: boolean | undefined;
}) {
  return (
    <section className={cn("panel flex min-h-0 flex-col overflow-hidden", className)}>
      {title && (
        <header className="flex h-9 shrink-0 items-center justify-between gap-2 border-b border-border px-3">
          <span className="section-label">{title}</span>
          {action}
        </header>
      )}
      <div className={cn(dense ? "" : "p-3", "min-h-0 flex-1 overflow-auto", bodyClassName)}>
        {children}
      </div>
    </section>
  );
}

export function Metric({
  label,
  value,
  hint,
  tone,
}: {
  label: string;
  value: ReactNode;
  hint?: ReactNode | undefined;
  tone?: Tone | undefined;
}) {
  return (
    <div className="min-w-0 px-3 py-2">
      <div className="section-label truncate">{label}</div>
      <div
        className={cn(
          "mt-1 truncate text-[15px] font-semibold tabular-nums",
          tone === "success" && "text-success",
          tone === "danger" && "text-destructive",
          tone === "warning" && "text-warning",
          tone === "running" && "text-running",
        )}
      >
        {value}
      </div>
      {hint && <div className="mt-0.5 truncate text-[11px] text-muted-foreground">{hint}</div>}
    </div>
  );
}

export function KV({ k, v }: { k: string; v: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-1">
      <span className="shrink-0 text-[11px] text-muted-foreground">{k}</span>
      <span className="min-w-0 truncate text-right text-[12px] font-medium">{v}</span>
    </div>
  );
}

export function Hint({ label, children }: { label: string; children: ReactNode }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent className="text-[11px]">{label}</TooltipContent>
    </Tooltip>
  );
}

export function EmptyState({
  icon,
  title,
  hint,
  action,
}: {
  icon?: ReactNode | undefined;
  title: string;
  hint?: string | undefined;
  action?: ReactNode | undefined;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 px-6 py-10 text-center">
      {icon && <div className="text-muted-foreground/60">{icon}</div>}
      <p className="text-[13px] font-medium">{title}</p>
      {hint && <p className="max-w-sm text-[11.5px] text-muted-foreground">{hint}</p>}
      {action}
    </div>
  );
}

/* ---------------- domain badges ---------------- */

export const stageTone: Record<string, Tone> = {
  planned: "muted",
  ready: "info",
  implementation: "primary",
  verification: "running",
  review: "warning",
  integration: "success",
  waiting: "danger",
  done: "success",
};

export const priorityTone: Record<string, Tone> = {
  critical: "danger",
  high: "warning",
  medium: "info",
  low: "muted",
};

export const agentStateTone: Record<string, Tone> = {
  active: "success",
  working: "running",
  idle: "muted",
  suspended: "danger",
  blocked: "warning",
};

export function StageBadge({ stage }: { stage: string }) {
  return (
    <Chip tone={stageTone[stage] ?? "muted"} className="uppercase tracking-wide">
      {stage}
    </Chip>
  );
}
