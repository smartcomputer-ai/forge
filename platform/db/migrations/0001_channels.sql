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
CREATE TABLE "channel_identities" (
	"id" uuid PRIMARY KEY DEFAULT gen_random_uuid() NOT NULL,
	"user_id" text NOT NULL,
	"channel" text NOT NULL,
	"handle" text NOT NULL,
	"display_name" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "channel_identities" ADD CONSTRAINT "channel_identities_user_id_user_id_fk" FOREIGN KEY ("user_id") REFERENCES "public"."user"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE UNIQUE INDEX "channel_accounts_provider_account_idx" ON "channel_accounts" USING btree ("provider","account_id");--> statement-breakpoint
CREATE UNIQUE INDEX "channel_identities_channel_handle_idx" ON "channel_identities" USING btree ("channel","handle");
