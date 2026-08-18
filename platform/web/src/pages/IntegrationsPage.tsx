import { useState } from "react";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { LoadingNote, PageHeader, UniverseNotFound } from "@/components/page";
import { AddIntegrationDialog } from "@/components/integrations/add-integration-dialog";
import { IntegrationDetailsDialog } from "@/components/integrations/integration-details-dialog";
import { IntegrationList } from "@/components/integrations/integration-list";
import {
  useIntegrations,
  useInvalidateIntegrations,
  type ConnectedIntegration,
} from "@/components/integrations/use-integrations";
import { canManage, useActiveUniverse } from "@/lib/universes";

export function IntegrationsPage({ admin }: { admin: boolean }) {
  const { universe, slug, isLoading } = useActiveUniverse();
  if (isLoading) return <LoadingNote />;
  if (!universe || !canManage(universe, admin)) return <UniverseNotFound slug={slug} />;
  return <Integrations universeId={universe.id} />;
}

function Integrations({ universeId }: { universeId: string }) {
  const [addOpen, setAddOpen] = useState(false);
  const [selected, setSelected] = useState<ConnectedIntegration | null>(null);
  const { connected, isLoading, error } = useIntegrations(universeId);
  const invalidate = useInvalidateIntegrations(universeId);

  // Keep the details dialog on the freshest copy of its integration.
  const current = selected ? connected.find((i) => i.id === selected.id) ?? selected : null;

  return (
    <>
      <PageHeader
        title="Integrations"
        description="Third-party services connected to this universe — credentials stay encrypted and are never returned."
        actions={
          <Button onClick={() => setAddOpen(true)}>
            <Plus data-icon="inline-start" />
            Add integration
          </Button>
        }
      />
      {isLoading && <LoadingNote />}
      {error && <p className="text-sm text-destructive">{error.message}</p>}
      {!isLoading && <IntegrationList integrations={connected} onSelect={setSelected} />}
      <AddIntegrationDialog
        universeId={universeId}
        open={addOpen}
        onOpenChange={setAddOpen}
        onAdded={() => void invalidate()}
      />
      <IntegrationDetailsDialog
        universeId={universeId}
        integration={current}
        onOpenChange={(open) => {
          if (!open) setSelected(null);
        }}
        onChanged={() => void invalidate()}
      />
    </>
  );
}
