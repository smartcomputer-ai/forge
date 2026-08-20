ALTER TABLE "bot_events" ADD COLUMN "trigger_id" uuid;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "session" jsonb;--> statement-breakpoint
ALTER TABLE "bot_triggers" ADD COLUMN "coalesce" jsonb;--> statement-breakpoint
ALTER TABLE "bot_triggers" ADD COLUMN "deliver" jsonb;--> statement-breakpoint
ALTER TABLE "bots" ADD COLUMN "breaker" jsonb;--> statement-breakpoint
ALTER TABLE "bot_events" ADD CONSTRAINT "bot_events_trigger_id_bot_triggers_id_fk" FOREIGN KEY ("trigger_id") REFERENCES "public"."bot_triggers"("id") ON DELETE set null ON UPDATE no action;