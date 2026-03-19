CREATE TABLE "social_connections" (
	"id" text PRIMARY KEY NOT NULL,
	"platform" varchar(30) NOT NULL,
	"access_token" text NOT NULL,
	"refresh_token" text,
	"token_expires_at" timestamp with time zone,
	"scopes" text[],
	"platform_user_id" text,
	"platform_username" text,
	"platform_avatar" text,
	"platform_meta" jsonb,
	"status" varchar(20) DEFAULT 'active',
	"last_used_at" timestamp with time zone,
	"last_refreshed_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
