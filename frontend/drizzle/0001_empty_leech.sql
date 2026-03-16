CREATE TABLE "hf_datasets" (
	"id" text PRIMARY KEY NOT NULL,
	"hf_id" text NOT NULL,
	"author" text,
	"name" text NOT NULL,
	"description" text,
	"tags" text[],
	"downloads" integer DEFAULT 0,
	"likes" integer DEFAULT 0,
	"config" text,
	"splits" jsonb,
	"num_rows" integer DEFAULT 0,
	"columns" text[],
	"features" jsonb,
	"cache_path" text,
	"size_bytes" bigint DEFAULT 0,
	"status" varchar(20) DEFAULT 'available',
	"error_message" text,
	"downloaded_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "integration_connections" (
	"id" text PRIMARY KEY NOT NULL,
	"integration_id" text NOT NULL,
	"name" text NOT NULL,
	"config" jsonb,
	"status" varchar(20) DEFAULT 'active',
	"last_tested_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "integrations" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"description" text,
	"category" varchar(30) NOT NULL,
	"icon" text,
	"color" text,
	"auth_type" varchar(20) DEFAULT 'api_key',
	"config_fields" jsonb,
	"base_url" text,
	"docs_url" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "plugins" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"description" text,
	"version" varchar(20) DEFAULT '1.0.0',
	"author" text,
	"source" varchar(20) DEFAULT 'personal' NOT NULL,
	"status" varchar(20) DEFAULT 'active',
	"icon" text,
	"color" text,
	"category" text,
	"capabilities" text[],
	"config_schema" jsonb,
	"config" jsonb,
	"installed" boolean DEFAULT false,
	"downloads" integer DEFAULT 0,
	"rating" numeric(3, 2),
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "workflow_steps" ADD COLUMN "system_instructions" text;--> statement-breakpoint
ALTER TABLE "workflow_steps" ADD COLUMN "voice_command" text;--> statement-breakpoint
ALTER TABLE "workflow_steps" ADD COLUMN "file_locations" text[];--> statement-breakpoint
ALTER TABLE "workflow_steps" ADD COLUMN "step_file_urls" text[];--> statement-breakpoint
ALTER TABLE "workflow_steps" ADD COLUMN "step_image_urls" text[];--> statement-breakpoint
ALTER TABLE "workflow_steps" ADD COLUMN "integrations" jsonb;