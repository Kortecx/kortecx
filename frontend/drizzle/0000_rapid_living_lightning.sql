CREATE TABLE "alerts" (
	"id" text PRIMARY KEY NOT NULL,
	"severity" varchar(20) NOT NULL,
	"title" text NOT NULL,
	"message" text NOT NULL,
	"provider_id" text,
	"expert_id" text,
	"acknowledged" boolean DEFAULT false,
	"acknowledged_at" timestamp with time zone,
	"resolved_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "datasets" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"description" text,
	"status" varchar(20) DEFAULT 'draft' NOT NULL,
	"format" varchar(20) DEFAULT 'jsonl',
	"sample_count" integer DEFAULT 0,
	"size_bytes" bigint DEFAULT 0,
	"quality_score" integer,
	"tags" text[],
	"categories" text[],
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "experts" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"description" text,
	"role" varchar(30) NOT NULL,
	"status" varchar(20) DEFAULT 'idle' NOT NULL,
	"version" varchar(20) DEFAULT '1.0.0',
	"model_id" text NOT NULL,
	"model_name" text,
	"provider_id" text NOT NULL,
	"provider_name" text,
	"model_source" varchar(20) DEFAULT 'provider',
	"local_model_config" jsonb,
	"system_prompt" text,
	"temperature" numeric(3, 2) DEFAULT '0.7',
	"max_tokens" integer DEFAULT 4096,
	"total_runs" integer DEFAULT 0,
	"success_rate" numeric(5, 4) DEFAULT '0',
	"avg_latency_ms" integer DEFAULT 0,
	"avg_cost_per_run" numeric(8, 4) DEFAULT '0',
	"rating" numeric(3, 2) DEFAULT '0',
	"tags" text[],
	"is_public" boolean DEFAULT false,
	"is_finetuned" boolean DEFAULT false,
	"replica_count" integer DEFAULT 1,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "logs" (
	"id" serial PRIMARY KEY NOT NULL,
	"timestamp" timestamp with time zone DEFAULT now() NOT NULL,
	"level" varchar(10) NOT NULL,
	"message" text NOT NULL,
	"source" text,
	"metadata" jsonb,
	"task_id" text,
	"run_id" text
);
--> statement-breakpoint
CREATE TABLE "metrics" (
	"id" serial PRIMARY KEY NOT NULL,
	"captured_at" timestamp with time zone DEFAULT now() NOT NULL,
	"active_agents" integer DEFAULT 0,
	"tasks_completed" integer DEFAULT 0,
	"tokens_used" bigint DEFAULT 0,
	"avg_latency_ms" integer DEFAULT 0,
	"success_rate" numeric(5, 4) DEFAULT '0',
	"cost_usd" numeric(10, 4) DEFAULT '0',
	"error_count" integer DEFAULT 0
);
--> statement-breakpoint
CREATE TABLE "projects" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"description" text,
	"status" varchar(20) DEFAULT 'active',
	"platforms" text[],
	"posts_count" integer DEFAULT 0,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "tasks" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"workflow_id" text,
	"workflow_name" text,
	"status" varchar(20) DEFAULT 'queued' NOT NULL,
	"priority" varchar(20) DEFAULT 'normal',
	"current_step" integer DEFAULT 0,
	"total_steps" integer DEFAULT 1,
	"current_expert" text,
	"tokens_used" integer DEFAULT 0,
	"estimated_tokens" integer,
	"progress" integer DEFAULT 0,
	"input" text,
	"output" text,
	"error_message" text,
	"started_at" timestamp with time zone,
	"completed_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "training_jobs" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"expert_id" text,
	"base_model_id" text NOT NULL,
	"dataset_id" text,
	"status" varchar(20) DEFAULT 'queued' NOT NULL,
	"progress" integer DEFAULT 0,
	"epochs" integer DEFAULT 3,
	"current_epoch" integer DEFAULT 0,
	"learning_rate" numeric(10, 8),
	"batch_size" integer DEFAULT 16,
	"training_samples" integer,
	"validation_samples" integer,
	"eval_loss" numeric(8, 6),
	"eval_accuracy" numeric(5, 4),
	"gpu_hours" numeric(8, 2),
	"cost_usd" numeric(8, 2),
	"started_at" timestamp with time zone,
	"completed_at" timestamp with time zone,
	"estimated_completion_at" timestamp with time zone,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "workflow_runs" (
	"id" text PRIMARY KEY NOT NULL,
	"workflow_id" text NOT NULL,
	"workflow_name" text NOT NULL,
	"status" varchar(20) NOT NULL,
	"started_at" timestamp with time zone,
	"completed_at" timestamp with time zone,
	"total_tokens_used" integer DEFAULT 0,
	"total_cost_usd" numeric(10, 4) DEFAULT '0',
	"duration_sec" integer,
	"input" text,
	"expert_chain" text[],
	"error_message" text,
	"metadata" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "workflow_steps" (
	"id" text PRIMARY KEY NOT NULL,
	"workflow_id" text NOT NULL,
	"step_order" integer NOT NULL,
	"expert_id" text,
	"task_description" text NOT NULL,
	"model_source" varchar(20) DEFAULT 'provider' NOT NULL,
	"local_model_config" jsonb,
	"connection_type" varchar(20) DEFAULT 'sequential',
	"temperature" numeric(3, 2) DEFAULT '0.7',
	"max_tokens" integer DEFAULT 4096,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "workflows" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"description" text,
	"goal_statement" text,
	"goal_file_url" text,
	"input_file_urls" text[],
	"status" varchar(20) DEFAULT 'draft',
	"estimated_tokens" integer,
	"estimated_cost_usd" numeric(8, 4),
	"estimated_duration_sec" integer,
	"total_runs" integer DEFAULT 0,
	"successful_runs" integer DEFAULT 0,
	"tags" text[],
	"is_template" boolean DEFAULT false,
	"template_category" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL,
	"last_run_at" timestamp with time zone
);
