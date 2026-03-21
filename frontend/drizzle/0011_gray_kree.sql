CREATE TABLE "execution_audit" (
	"id" text PRIMARY KEY NOT NULL,
	"run_id" text NOT NULL,
	"workflow_id" text,
	"workflow_name" text,
	"agent_id" text,
	"step_id" text,
	"expert_id" text,
	"phase" varchar(30),
	"operation" varchar(30) NOT NULL,
	"prompt" text,
	"response" text,
	"tokens_used" integer DEFAULT 0,
	"duration_ms" integer DEFAULT 0,
	"model" text,
	"engine" varchar(20),
	"temperature" numeric(3, 2),
	"status" varchar(20) DEFAULT 'ok',
	"error" text,
	"metadata" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "model_comparisons" (
	"id" text PRIMARY KEY NOT NULL,
	"run_id" text NOT NULL,
	"step_id" text NOT NULL,
	"original_model" text NOT NULL,
	"original_engine" varchar(20),
	"original_tokens" integer DEFAULT 0,
	"original_duration_ms" integer DEFAULT 0,
	"original_response" text,
	"comparison_model" text NOT NULL,
	"comparison_engine" varchar(20),
	"comparison_tokens" integer DEFAULT 0,
	"comparison_duration_ms" integer DEFAULT 0,
	"comparison_response" text,
	"temperature" numeric(3, 2),
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "project_assets" (
	"id" text PRIMARY KEY NOT NULL,
	"project_id" text NOT NULL,
	"asset_type" varchar(30) NOT NULL,
	"asset_id" text NOT NULL,
	"asset_name" text NOT NULL,
	"asset_path" text,
	"mlflow_run_id" text,
	"metadata" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "step_executions" (
	"id" text PRIMARY KEY NOT NULL,
	"run_id" text NOT NULL,
	"workflow_id" text,
	"step_id" text NOT NULL,
	"agent_id" text,
	"expert_id" text,
	"step_name" text,
	"status" varchar(20) DEFAULT 'pending' NOT NULL,
	"model" text,
	"engine" varchar(20),
	"tokens_used" integer DEFAULT 0,
	"prompt_tokens" integer DEFAULT 0,
	"completion_tokens" integer DEFAULT 0,
	"duration_ms" integer DEFAULT 0,
	"queue_wait_ms" integer DEFAULT 0,
	"inference_ms" integer DEFAULT 0,
	"cpu_percent" numeric(5, 2) DEFAULT '0',
	"gpu_percent" numeric(5, 2) DEFAULT '0',
	"memory_mb" numeric(8, 2) DEFAULT '0',
	"prompt_preview" text,
	"response_preview" text,
	"error_message" text,
	"metadata" jsonb,
	"started_at" timestamp with time zone,
	"completed_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "workflow_steps" ADD COLUMN "name" text;--> statement-breakpoint
ALTER TABLE "workflow_steps" ADD COLUMN "step_description" text;--> statement-breakpoint
ALTER TABLE "workflow_steps" ADD COLUMN "share_memory" boolean DEFAULT true;--> statement-breakpoint
ALTER TABLE "workflows" ADD COLUMN "metadata" jsonb;