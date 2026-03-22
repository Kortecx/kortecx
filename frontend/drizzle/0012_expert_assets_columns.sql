ALTER TABLE "assets" ADD COLUMN "expert_id" text;--> statement-breakpoint
ALTER TABLE "assets" ADD COLUMN "expert_run_id" text;--> statement-breakpoint
ALTER TABLE "assets" ADD COLUMN "source_type" varchar(20);--> statement-breakpoint
CREATE TABLE "expert_runs" (
	"id" text PRIMARY KEY NOT NULL,
	"expert_id" text NOT NULL,
	"expert_name" text NOT NULL,
	"status" varchar(20) DEFAULT 'running' NOT NULL,
	"model" text,
	"engine" varchar(20),
	"temperature" numeric(3, 2),
	"max_tokens" integer,
	"system_prompt" text,
	"user_prompt" text,
	"response_text" text,
	"tokens_used" integer DEFAULT 0,
	"duration_ms" integer DEFAULT 0,
	"artifact_count" integer DEFAULT 0,
	"error_message" text,
	"metadata" jsonb,
	"started_at" timestamp with time zone,
	"completed_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
