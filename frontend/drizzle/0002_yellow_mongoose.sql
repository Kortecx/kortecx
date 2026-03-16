CREATE TABLE "api_keys" (
	"id" text PRIMARY KEY NOT NULL,
	"provider_id" text NOT NULL,
	"key_hash" text NOT NULL,
	"key_prefix" text,
	"key_suffix" text,
	"encrypted_key" text NOT NULL,
	"status" varchar(20) DEFAULT 'active',
	"last_used_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
