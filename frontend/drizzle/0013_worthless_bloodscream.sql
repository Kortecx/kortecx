DROP TABLE "training_jobs" CASCADE;--> statement-breakpoint
ALTER TABLE "experts" ADD COLUMN "category" varchar(30) DEFAULT 'custom';--> statement-breakpoint
ALTER TABLE "experts" ADD COLUMN "complexity_level" integer DEFAULT 3;--> statement-breakpoint
ALTER TABLE "model_comparisons" ADD COLUMN "prompt" text;--> statement-breakpoint
ALTER TABLE "model_comparisons" ADD COLUMN "system_prompt" text;--> statement-breakpoint
ALTER TABLE "model_comparisons" ADD COLUMN "document_names" text[];--> statement-breakpoint
ALTER TABLE "model_comparisons" ADD COLUMN "document_content" text;--> statement-breakpoint
ALTER TABLE "model_comparisons" ADD COLUMN "original_tokens_per_sec" numeric(8, 1);--> statement-breakpoint
ALTER TABLE "model_comparisons" ADD COLUMN "comparison_tokens_per_sec" numeric(8, 1);--> statement-breakpoint
ALTER TABLE "tasks" ADD COLUMN "expert_id" text;--> statement-breakpoint
ALTER TABLE "tasks" ADD COLUMN "expert_run_id" text;--> statement-breakpoint
ALTER TABLE "workflow_steps" ADD COLUMN "step_type" varchar(20) DEFAULT 'agent';--> statement-breakpoint
ALTER TABLE "workflow_steps" ADD COLUMN "action_config" jsonb;