CREATE TABLE "channel_accounts" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"provider" text NOT NULL,
	"account_id" text NOT NULL,
	"display_name" text NOT NULL,
	"credential_ref" text,
	"state_ref" text,
	"settings" jsonb DEFAULT '{}'::jsonb NOT NULL,
	"enabled" boolean DEFAULT true NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "channel_bindings" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"universe_id" uuid NOT NULL,
	"channel_account_id" uuid NOT NULL,
	"name" text NOT NULL,
	"match_scope" text,
	"profile_id" text,
	"session_key" text NOT NULL,
	"activation" jsonb,
	"access" jsonb,
	"pairing_code" text,
	"priority" integer DEFAULT 100 NOT NULL,
	"enabled" boolean DEFAULT true NOT NULL,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "channel_bindings_id_account_unique" UNIQUE("id","channel_account_id")
);
--> statement-breakpoint
CREATE TABLE "channel_identities" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" text NOT NULL,
	"channel" text NOT NULL,
	"handle" text NOT NULL,
	"display_name" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "channel_pairings" (
	"key" text PRIMARY KEY NOT NULL,
	"binding_id" uuid NOT NULL,
	"channel_account_id" uuid NOT NULL,
	"chat_id" text NOT NULL,
	"paired_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "channel_bindings" ADD CONSTRAINT "channel_bindings_universe_id_universes_id_fk" FOREIGN KEY ("universe_id") REFERENCES "public"."universes"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "channel_bindings" ADD CONSTRAINT "channel_bindings_channel_account_id_channel_accounts_id_fk" FOREIGN KEY ("channel_account_id") REFERENCES "public"."channel_accounts"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "channel_identities" ADD CONSTRAINT "channel_identities_user_id_user_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "channel_pairings" ADD CONSTRAINT "channel_pairings_channel_account_id_channel_accounts_id_fk" FOREIGN KEY ("channel_account_id") REFERENCES "public"."channel_accounts"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "channel_pairings" ADD CONSTRAINT "channel_pairings_binding_account_fk" FOREIGN KEY ("binding_id","channel_account_id") REFERENCES "public"."channel_bindings"("id","channel_account_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE UNIQUE INDEX "channel_accounts_provider_account_idx" ON "channel_accounts" USING btree ("provider","account_id");--> statement-breakpoint
CREATE INDEX "channel_bindings_universe_idx" ON "channel_bindings" USING btree ("universe_id");--> statement-breakpoint
CREATE INDEX "channel_bindings_channel_account_idx" ON "channel_bindings" USING btree ("channel_account_id");--> statement-breakpoint
CREATE UNIQUE INDEX "channel_bindings_universe_name_idx" ON "channel_bindings" USING btree ("universe_id","name");--> statement-breakpoint
CREATE UNIQUE INDEX "channel_identities_channel_handle_idx" ON "channel_identities" USING btree ("channel","handle");--> statement-breakpoint
CREATE INDEX "channel_pairings_account_chat_idx" ON "channel_pairings" USING btree ("channel_account_id","chat_id");