DROP TABLE "bot_activity" CASCADE;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "outcome" text;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "outcome_detail" text;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "delivery_id" text;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "run_id" text;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "resolved_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "bot_triggers" ADD COLUMN "disabled_reason" text;--> statement-breakpoint
ALTER TABLE "bot_triggers" ADD COLUMN "disabled_at" timestamp with time zone;--> statement-breakpoint
ALTER TABLE "bot_triggers" ADD COLUMN "last_filter_error" text;--> statement-breakpoint
ALTER TABLE "bot_triggers" ADD COLUMN "last_filter_error_at" timestamp with time zone;