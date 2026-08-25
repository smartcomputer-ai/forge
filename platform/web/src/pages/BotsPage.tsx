import { useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { Plus } from "lucide-react";
import { NavLink, useParams } from "react-router-dom";
import { api, type Bot, type BotListItem, type BotLineage,
  type BotState } from "@/api";
import { BotDetail } from "@/components/bot/detail";
import { CreateBotDialog } from "@/components/bot/create-bot-dialog";
import { BotFace } from "@/components/icons/bot";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { LoadingNote, UniverseNotFound } from "@/components/page";
import { canManage, useActiveUniverse } from "@/lib/universes";
import { cn } from "@/lib/utils";

export function BotsPage({ admin }: { admin: boolean }) {
  const { universe, slug, isLoading } = useActiveUniverse();
  const { botId } = useParams<{ botId?: string }>();

  if (isLoading) return <LoadingNote />;
  if (!universe) {
    return (
      <div className="p-6">
        <UniverseNotFound slug={slug} />
      </div>
    );
  }

  const manage = canManage(universe, admin);
  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      <aside
        className={cn(
          "w-full shrink-0 flex-col border-r md:flex md:w-80",
          botId ? "hidden" : "flex",
        )}
      >
        <BotsPane universeId={universe.id} slug={slug!} activeId={botId} manage={manage} />
      </aside>
      <section className={cn("min-w-0 flex-1 flex-col", botId ? "flex" : "hidden md:flex")}>
        {botId ? (
          <BotWorkspace key={botId} slug={slug!} botId={botId} manage={manage} />
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-sm text-muted-foreground">
            <BotFace size={40} className="text-muted-foreground/60" />
            Select a bot{manage ? ", or create one." : "."}
          </div>
        )}
      </section>
    </div>
  );
}

function BotsPane({
  universeId,
  slug,
  activeId,
  manage,
}: {
  universeId: string;
  slug: string;
  activeId: string | undefined;
  manage: boolean;
}) {
  const bots = useQuery({
    queryKey: ["bots", universeId],
    queryFn: () => api<{ bots: BotListItem[] }>("GET", `/api/v1/universes/${universeId}/bots`),
  });
  const [createOpen, setCreateOpen] = useState(false);

  return (
    <>
      <div className="flex h-12 shrink-0 items-center gap-2 border-b px-4">
        <h1 className="text-sm font-semibold">Bots</h1>
        {bots.data && <span className="text-xs text-muted-foreground">{bots.data.bots.length}</span>}
        {manage && (
          <Button
            variant="ghost"
            size="icon-sm"
            className="ml-auto"
            onClick={() => setCreateOpen(true)}
            aria-label="Create bot"
          >
            <Plus />
          </Button>
        )}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {bots.isLoading && <p className="p-4 text-sm text-muted-foreground">Loading…</p>}
        {bots.error && <p className="p-4 text-sm text-destructive">{bots.error.message}</p>}
        {bots.data?.bots.length === 0 && (
          <p className="p-4 text-sm text-muted-foreground">
            No bots yet{manage ? " — create one and give it a schedule." : "."}
          </p>
        )}
        <ul>
          {bots.data?.bots.map((bot) => {
            const preview = bot.brief?.trim() || "No standing brief";
            return (
              <li key={bot.id}>
                <NavLink
                  to={`/u/${slug}/bots/${bot.id}`}
                  className={cn(
                    "block min-w-0 border-b px-4 py-3 text-sm hover:bg-muted/50",
                    bot.id === activeId && "bg-muted",
                  )}
                >
                  <span className="flex min-w-0 items-center gap-2">
                    <span className="min-w-0 truncate font-medium">{bot.name}</span>
                    <Badge variant="secondary" className="max-w-28 truncate" title={`Profile: ${bot.profileId}`}>
                      {bot.profileId}
                    </Badge>
                    {!bot.enabled && <span className="shrink-0 text-xs text-destructive">Disabled</span>}
                  </span>
                  <span className="mt-0.5 flex min-w-0 gap-2 text-xs text-muted-foreground">
                    <span className="min-w-0 flex-1 truncate" title={preview}>{preview}</span>
                    <span className="shrink-0">
                      {bot.triggerCount} {bot.triggerCount === 1 ? "trigger" : "triggers"}
                    </span>
                  </span>
                </NavLink>
              </li>
            );
          })}
        </ul>
      </div>
      {manage && (
        <CreateBotDialog
          universeId={universeId}
          slug={slug}
          open={createOpen}
          onOpenChange={setCreateOpen}
        />
      )}
    </>
  );
}

function BotWorkspace({
  slug,
  botId,
  manage,
}: {
  slug: string;
  botId: string;
  manage: boolean;
}) {
  const bot = useQuery({
    queryKey: ["bot", botId],
    queryFn: () => api<{ bot: Bot }>("GET", `/api/v1/bots/${botId}`),
  });
  const state = useQuery({
    queryKey: ["bot-state", botId],
    queryFn: () => api<{ state: BotState; lineage?: BotLineage }>("GET", `/api/v1/bots/${botId}/state`),
    refetchInterval: 3_000,
    retry: true,
  });

  if (bot.isLoading) return <DetailNote>Loading…</DetailNote>;
  if (bot.error || !bot.data) {
    return <DetailNote destructive>{bot.error?.message ?? "Bot not found"}</DetailNote>;
  }

  return (
    <BotDetail
      slug={slug}
      bot={bot.data.bot}
      state={state.data?.state}
      {...(state.data?.lineage ? { lineage: state.data.lineage } : {})}
      {...(state.error ? { stateError: state.error.message } : {})}
      manage={manage}
    />
  );
}

function DetailNote({ children, destructive = false }: { children: ReactNode; destructive?: boolean }) {
  return (
    <div
      className={cn(
        "flex flex-1 items-center justify-center p-6 text-sm text-muted-foreground",
        destructive && "text-destructive",
      )}
    >
      {children}
    </div>
  );
}
