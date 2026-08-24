ALTER TABLE "bot_events" ADD COLUMN "seq" bigint;--> statement-breakpoint
ALTER TABLE "bot_events" ADD COLUMN "prompt_ref" text;--> statement-breakpoint
ALTER TABLE "bots" ADD COLUMN "event_seq" bigint DEFAULT 0 NOT NULL;--> statement-breakpoint
CREATE UNIQUE INDEX "bot_events_bot_seq_idx" ON "bot_events" USING btree ("bot_id","seq");--> statement-breakpoint
UPDATE "bot_events" SET "seq" = numbered.rn FROM (SELECT "id", row_number() OVER (PARTITION BY "bot_id" ORDER BY "created_at", "id") AS rn FROM "bot_events") AS numbered WHERE "bot_events"."id" = numbered."id";--> statement-breakpoint
UPDATE "bots" SET "event_seq" = COALESCE((SELECT max("seq") FROM "bot_events" WHERE "bot_events"."bot_id" = "bots"."id"), 0);