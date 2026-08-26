DROP TABLE "channel_bindings" CASCADE;--> statement-breakpoint
DROP TABLE "channel_pairings" CASCADE;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "media" jsonb;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "tools" text;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "notify" jsonb;--> statement-breakpoint
ALTER TABLE "bot_triggers" ADD COLUMN "session_ttl_ms" integer;--> statement-breakpoint
CREATE TABLE "channel_pairings" (
	"key" text PRIMARY KEY NOT NULL,
	"trigger_id" uuid NOT NULL,
	"channel_account_id" uuid NOT NULL,
	"chat_id" text NOT NULL,
	"paired_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "channel_pairings" ADD CONSTRAINT "channel_pairings_trigger_id_bot_triggers_id_fk" FOREIGN KEY ("trigger_id") REFERENCES "public"."bot_triggers"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "channel_pairings" ADD CONSTRAINT "channel_pairings_channel_account_id_channel_accounts_id_fk" FOREIGN KEY ("channel_account_id") REFERENCES "public"."channel_accounts"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "channel_pairings_account_chat_idx" ON "channel_pairings" USING btree ("channel_account_id","chat_id");--> statement-breakpoint
CREATE INDEX "channel_pairings_trigger_idx" ON "channel_pairings" USING btree ("trigger_id");
