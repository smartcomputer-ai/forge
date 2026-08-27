CREATE TABLE "bots" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"universe_id" uuid NOT NULL,
	"name" text NOT NULL,
	"display_name" text,
	"description" text,
	"profile_id" text NOT NULL,
	"brief" text,
	"runs_per_day" integer,
	"breaker" jsonb,
	"routed_session_ttl_ms" integer,
	"event_seq" bigint DEFAULT 0 NOT NULL,
	"self_config" boolean DEFAULT false NOT NULL,
	"emit" boolean DEFAULT false NOT NULL,
	"enabled" boolean DEFAULT true NOT NULL,
	"closed_at" timestamp with time zone,
	"closed_sessions" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "bot_triggers" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"bot_id" uuid NOT NULL,
	"name" text NOT NULL,
	"kind" text NOT NULL,
	"spec" jsonb NOT NULL,
	"filter" text,
	"route" jsonb,
	"coalesce" jsonb,
	"deliver" jsonb,
	"cursor" jsonb,
	"session_ttl_ms" integer,
	"enabled" boolean DEFAULT true NOT NULL,
	"disabled_reason" text,
	"disabled_at" timestamp with time zone,
	"last_filter_error" text,
	"last_filter_error_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "bot_events" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"bot_id" uuid NOT NULL,
	"event_id" text NOT NULL,
	"seq" bigint,
	"trigger_id" uuid,
	"kind" text NOT NULL,
	"source" text NOT NULL,
	"occurred_at" timestamp with time zone NOT NULL,
	"ref" text NOT NULL,
	"prompt_ref" text,
	"session" jsonb,
	"sender_bot_id" uuid,
	"hops" integer DEFAULT 0 NOT NULL,
	"reply_to" jsonb,
	"in_reply_to" jsonb,
	"media" jsonb,
	"tools" text,
	"notify" jsonb,
	"outcome" text,
	"outcome_detail" text,
	"delivery_id" text,
	"run_id" text,
	"resolved_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "channel_pairings" (
	"key" text PRIMARY KEY NOT NULL,
	"trigger_id" uuid NOT NULL,
	"channel_account_id" uuid NOT NULL,
	"chat_id" text NOT NULL,
	"paired_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "bots" ADD CONSTRAINT "bots_universe_id_universes_id_fk" FOREIGN KEY ("universe_id") REFERENCES "public"."universes"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "bot_triggers" ADD CONSTRAINT "bot_triggers_bot_id_bots_id_fk" FOREIGN KEY ("bot_id") REFERENCES "public"."bots"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "bot_events" ADD CONSTRAINT "bot_events_bot_id_bots_id_fk" FOREIGN KEY ("bot_id") REFERENCES "public"."bots"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "bot_events" ADD CONSTRAINT "bot_events_trigger_id_bot_triggers_id_fk" FOREIGN KEY ("trigger_id") REFERENCES "public"."bot_triggers"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "bot_events" ADD CONSTRAINT "bot_events_sender_bot_id_bots_id_fk" FOREIGN KEY ("sender_bot_id") REFERENCES "public"."bots"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "channel_pairings" ADD CONSTRAINT "channel_pairings_trigger_id_bot_triggers_id_fk" FOREIGN KEY ("trigger_id") REFERENCES "public"."bot_triggers"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "channel_pairings" ADD CONSTRAINT "channel_pairings_channel_account_id_channel_accounts_id_fk" FOREIGN KEY ("channel_account_id") REFERENCES "public"."channel_accounts"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE UNIQUE INDEX "bots_universe_name_idx" ON "bots" USING btree ("universe_id","name");--> statement-breakpoint
CREATE UNIQUE INDEX "bot_triggers_bot_name_idx" ON "bot_triggers" USING btree ("bot_id","name");--> statement-breakpoint
CREATE UNIQUE INDEX "bot_events_bot_event_idx" ON "bot_events" USING btree ("bot_id","event_id");--> statement-breakpoint
CREATE UNIQUE INDEX "bot_events_bot_seq_idx" ON "bot_events" USING btree ("bot_id","seq");--> statement-breakpoint
CREATE INDEX "bot_events_bot_received_idx" ON "bot_events" USING btree ("bot_id","created_at");--> statement-breakpoint
CREATE INDEX "channel_pairings_account_chat_idx" ON "channel_pairings" USING btree ("channel_account_id","chat_id");--> statement-breakpoint
CREATE INDEX "channel_pairings_trigger_idx" ON "channel_pairings" USING btree ("trigger_id");--> statement-breakpoint
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'lightspeed_channels') THEN
    GRANT SELECT ON "member", "universes", "channel_accounts", "channel_identities"
      TO lightspeed_channels;
    GRANT SELECT, INSERT, UPDATE, DELETE
      ON "bots", "bot_triggers", "bot_events", "channel_pairings"
      TO lightspeed_channels;
  END IF;
END
$$;
