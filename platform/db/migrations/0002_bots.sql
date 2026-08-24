CREATE TABLE "bot_activity" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"bot_id" uuid NOT NULL,
	"kind" text NOT NULL,
	"event_id" text,
	"run_id" text,
	"detail" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
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
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
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
	"enabled" boolean DEFAULT true NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "bots" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"universe_id" uuid NOT NULL,
	"name" text NOT NULL,
	"profile_id" text NOT NULL,
	"brief" text,
	"runs_per_day" integer,
	"breaker" jsonb,
	"routed_session_ttl_ms" integer,
	"event_seq" bigint DEFAULT 0 NOT NULL,
	"self_config" boolean DEFAULT false NOT NULL,
	"self_emit" boolean DEFAULT false NOT NULL,
	"enabled" boolean DEFAULT true NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "bot_activity" ADD CONSTRAINT "bot_activity_bot_id_bots_id_fk" FOREIGN KEY ("bot_id") REFERENCES "public"."bots"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "bot_events" ADD CONSTRAINT "bot_events_bot_id_bots_id_fk" FOREIGN KEY ("bot_id") REFERENCES "public"."bots"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "bot_events" ADD CONSTRAINT "bot_events_trigger_id_bot_triggers_id_fk" FOREIGN KEY ("trigger_id") REFERENCES "public"."bot_triggers"("id") ON DELETE set null ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "bot_triggers" ADD CONSTRAINT "bot_triggers_bot_id_bots_id_fk" FOREIGN KEY ("bot_id") REFERENCES "public"."bots"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "bots" ADD CONSTRAINT "bots_universe_id_universes_id_fk" FOREIGN KEY ("universe_id") REFERENCES "public"."universes"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "bot_activity_bot_created_idx" ON "bot_activity" USING btree ("bot_id","created_at");--> statement-breakpoint
CREATE UNIQUE INDEX "bot_events_bot_event_idx" ON "bot_events" USING btree ("bot_id","event_id");--> statement-breakpoint
CREATE UNIQUE INDEX "bot_events_bot_seq_idx" ON "bot_events" USING btree ("bot_id","seq");--> statement-breakpoint
CREATE INDEX "bot_events_bot_received_idx" ON "bot_events" USING btree ("bot_id","created_at");--> statement-breakpoint
CREATE UNIQUE INDEX "bot_triggers_bot_name_idx" ON "bot_triggers" USING btree ("bot_id","name");--> statement-breakpoint
CREATE UNIQUE INDEX "bots_universe_name_idx" ON "bots" USING btree ("universe_id","name");