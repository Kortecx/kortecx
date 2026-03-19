CREATE TABLE "oauth_credentials" (
	"id" text PRIMARY KEY NOT NULL,
	"platform" varchar(30) NOT NULL,
	"client_id" text NOT NULL,
	"client_secret" text NOT NULL,
	"key_hash" text NOT NULL,
	"status" varchar(20) DEFAULT 'active',
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	CONSTRAINT "oauth_credentials_platform_unique" UNIQUE("platform")
);
