import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export function PageHeader({
  title,
  subtitle,
  actions,
  tabs,
}: {
  title: ReactNode;
  subtitle?: ReactNode | undefined;
  actions?: ReactNode | undefined;
  tabs?: ReactNode | undefined;
}) {
  return (
    <div className="shrink-0 border-b border-border bg-surface px-4 py-2.5">
      <div className="flex items-center gap-3">
        <div className="min-w-0">
          <h1 className="truncate text-[14px] font-semibold tracking-tight">{title}</h1>
          {subtitle && (
            <p className="mt-0.5 truncate text-[11.5px] text-muted-foreground">{subtitle}</p>
          )}
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-2">{actions}</div>
      </div>
      {tabs && <div className="mt-2">{tabs}</div>}
    </div>
  );
}

export function PageBody({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("p-4", className)}>{children}</div>;
}

export function Page({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("flex h-full min-h-0 flex-col", className)}>{children}</div>;
}
