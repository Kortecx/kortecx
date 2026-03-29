-- Alert Rules: configurable trigger-based notifications
CREATE TABLE IF NOT EXISTS "alert_rules" (
  "id" text PRIMARY KEY NOT NULL,
  "name" text NOT NULL,
  "description" text,
  "trigger_type" varchar(30) NOT NULL,
  "conditions" jsonb NOT NULL DEFAULT '{}',
  "notification_config" jsonb NOT NULL DEFAULT '{}',
  "severity" varchar(20) NOT NULL DEFAULT 'warning',
  "enabled" boolean DEFAULT true,
  "cooldown_minutes" integer DEFAULT 15,
  "last_triggered_at" timestamp with time zone,
  "created_at" timestamp with time zone DEFAULT now() NOT NULL,
  "updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
