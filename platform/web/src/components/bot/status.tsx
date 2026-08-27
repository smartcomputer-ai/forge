import type { ReactNode } from "react";
import type { BotState } from "@/api";
import { Badge } from "@/components/ui/badge";

export function BotStatusBadge({ status }: { status?: BotState["controllerStatus"] }) {
  if (!status) return <Badge variant="outline">starting</Badge>;
  if (status === "degraded") return <Badge variant="destructive">degraded</Badge>;
  if (status === "budget_exhausted") return <Badge variant="destructive">budget exhausted</Badge>;
  if (status === "idle") return <Badge variant="secondary">idle</Badge>;
  return <Badge variant="outline">{status.replaceAll("_", " ")}</Badge>;
}

/** A titled block of the bot workspace. */
export function DetailSection({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="grid min-w-0 content-start gap-3">
      <div className="flex min-w-0 items-start justify-between gap-3">
        <div className="grid min-w-0 flex-1 gap-0.5">
          <h2 className="text-sm font-semibold">{title}</h2>
          {description && <p className="text-xs text-muted-foreground">{description}</p>}
        </div>
      </div>
      {children}
    </section>
  );
}

export function KeyValue({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="grid grid-cols-[5rem_minmax(0,1fr)] gap-2 text-xs">
      <span className="text-muted-foreground">{label}</span>
      <span className="truncate">{value}</span>
    </div>
  );
}
