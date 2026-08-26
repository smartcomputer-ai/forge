import { Link } from "react-router-dom";
import { ArrowRight, MessageCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { LoadingNote, PageHeader, UniverseNotFound } from "@/components/page";
import { canManage, useActiveUniverse } from "@/lib/universes";

/// Chat connections are bot triggers of kind `chat`, edited on each bot's
/// page. This settings section only points there; provider accounts and
/// connector health live in the deployment admin.
export function ChannelsPage({ admin }: { admin: boolean }) {
  const { universe, slug, isLoading } = useActiveUniverse();

  if (isLoading) {
    return <LoadingNote />;
  }
  if (!universe) {
    return <UniverseNotFound slug={slug} />;
  }

  const writable = canManage(universe, admin);
  const botsHref = `/u/${slug}/bots`;

  return (
    <>
      <PageHeader
        title="Channels"
        description="Telegram and WhatsApp conversations reach this universe through its bots."
      />
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <MessageCircle className="size-4 text-muted-foreground" />
            Chat connections live on bots
          </CardTitle>
          <CardDescription>
            A chat connection is a bot trigger: it names the provider account, which
            conversations it serves (direct, group, or both), how group messages activate the
            bot, who may talk to it, and the pairing code a conversation sends once to connect.
            Every message becomes an event for the bot, answered in one session per
            conversation.
          </CardDescription>
        </CardHeader>
        <CardContent className="grid gap-3">
          <p className="text-sm text-muted-foreground">
            {writable
              ? "Open a bot and add a chat trigger to connect an account, or edit an existing one to rotate its pairing code."
              : "Ask a universe owner to add a chat trigger on the bot you want to reach; the pairing code is shown to owners only."}
          </p>
          <div>
            <Button variant="outline" size="sm" render={<Link to={botsHref} />}>
              Go to bots
              <ArrowRight data-icon="inline-end" />
            </Button>
          </div>
        </CardContent>
      </Card>
    </>
  );
}
