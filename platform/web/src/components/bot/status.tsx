import type { BotState } from "@/api";
import { Badge } from "@/components/ui/badge";

export function BotStatusBadge({ status }: { status?: BotState["controllerStatus"] }) {
  if (!status) return <Badge variant="outline">starting</Badge>;
  if (status === "degraded") return <Badge variant="destructive">degraded</Badge>;
  if (status === "budget_exhausted") return <Badge variant="destructive">budget exhausted</Badge>;
  if (status === "idle") return <Badge variant="secondary">idle</Badge>;
  return <Badge variant="outline">{status.replaceAll("_", " ")}</Badge>;
}

export function PanelHeading({ title }: { title: string }) {
  return <h2 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">{title}</h2>;
}

export function KeyValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid grid-cols-[5rem_minmax(0,1fr)] gap-2 text-xs">
      <span className="text-muted-foreground">{label}</span>
      <span className="truncate">{value}</span>
    </div>
  );
}
