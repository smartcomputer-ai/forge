ALTER TABLE "bot_triggers" ADD COLUMN "spec" jsonb;--> statement-breakpoint
ALTER TABLE "bot_triggers" ADD COLUMN "filter" text;--> statement-breakpoint
ALTER TABLE "bot_triggers" ADD COLUMN "route" jsonb;--> statement-breakpoint
UPDATE "bot_triggers" SET "spec" = jsonb_build_object('cron', "cron", 'timezone', "timezone", 'summary', "summary") WHERE "kind" = 'schedule';--> statement-breakpoint
ALTER TABLE "bot_triggers" ALTER COLUMN "spec" SET NOT NULL;--> statement-breakpoint
ALTER TABLE "bot_triggers" DROP COLUMN "cron";--> statement-breakpoint
ALTER TABLE "bot_triggers" DROP COLUMN "timezone";--> statement-breakpoint
ALTER TABLE "bot_triggers" DROP COLUMN "summary";
