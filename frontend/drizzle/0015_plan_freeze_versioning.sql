-- Plan freeze & versioning columns
ALTER TABLE "workflows" ADD COLUMN "plan_frozen" boolean DEFAULT false;
ALTER TABLE "workflows" ADD COLUMN "frozen_plan_id" text;
ALTER TABLE "workflows" ADD COLUMN "active_plan_id" text;
ALTER TABLE "workflows" ADD COLUMN "plan_max_versions" integer DEFAULT 3;

ALTER TABLE "plans" ADD COLUMN "version" integer DEFAULT 1;
ALTER TABLE "plans" ADD COLUMN "plan_type" varchar(10) DEFAULT 'live';
ALTER TABLE "plans" ADD COLUMN "markdown_content" text;
ALTER TABLE "plans" ADD COLUMN "source_type" varchar(20) DEFAULT 'manual';
ALTER TABLE "plans" ADD COLUMN "frozen_at" timestamp with time zone;
