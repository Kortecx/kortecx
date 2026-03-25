ALTER TABLE "workflow_steps" ADD COLUMN "step_type" varchar(20) DEFAULT 'agent';
ALTER TABLE "workflow_steps" ADD COLUMN "action_config" jsonb;
