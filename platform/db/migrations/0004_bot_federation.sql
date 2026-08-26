ALTER TABLE "bot_events" ADD COLUMN "sender_bot_id" uuid;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "hops" integer DEFAULT 0 NOT NULL;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "reply_to" jsonb;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "in_reply_to" jsonb;--> statement-breakpoint
ALTER TABLE "bot_events" ADD CONSTRAINT "bot_events_sender_bot_id_bots_id_fk" FOREIGN KEY ("sender_bot_id") REFERENCES "public"."bots"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "bots" RENAME COLUMN "self_emit" TO "emit";
