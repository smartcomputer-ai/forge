import { ChevronRight } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCard,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { integrationDefinition } from "./catalog";
import type { ConnectedIntegration, IntegrationStatus } from "./use-integrations";

/// Connected integrations; a row opens the details dialog.
export function IntegrationList({
  integrations,
  onSelect,
}: {
  integrations: ConnectedIntegration[];
  onSelect: (integration: ConnectedIntegration) => void;
}) {
  if (integrations.length === 0) {
    return (
      <p className="rounded-xl border border-dashed p-5 text-sm text-muted-foreground">
        No integrations yet. Use <span className="font-medium">Add integration</span> to connect a
        GitHub App or a coding-agent subscription.
      </p>
    );
  }
  return (
    <TableCard>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead>Integration</TableHead>
            <TableHead>Type</TableHead>
            <TableHead>Status</TableHead>
            <TableHead className="w-0" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {integrations.map((integration) => {
            const definition = integrationDefinition(integration.kind);
            return (
              <TableRow
                key={integration.id}
                className="cursor-pointer"
                onClick={() => onSelect(integration)}
              >
                <TableCell>
                  <div className="flex items-center gap-3">
                    <span className="shrink-0 text-foreground">
                      <definition.Logo size={20} />
                    </span>
                    <div className="grid gap-0.5">
                      <span className="font-medium">{integration.title}</span>
                      <span className="text-xs text-muted-foreground">{integration.subtitle}</span>
                    </div>
                  </div>
                </TableCell>
                <TableCell className="text-muted-foreground">{definition.name}</TableCell>
                <TableCell>
                  <StatusBadge status={integration.status} />
                </TableCell>
                <TableCell className="text-muted-foreground">
                  <ChevronRight className="size-4" />
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </TableCard>
  );
}

function StatusBadge({ status }: { status: IntegrationStatus }) {
  if (status === "active") return <Badge variant="secondary">active</Badge>;
  if (status === "disabled") return <Badge variant="outline">disabled</Badge>;
  return (
    <Badge variant="outline" className="border-destructive/50 text-destructive">
      needs attention
    </Badge>
  );
}
