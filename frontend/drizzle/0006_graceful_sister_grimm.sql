CREATE TABLE "dataset_schemas" (
	"id" text PRIMARY KEY NOT NULL,
	"dataset_id" text,
	"name" text NOT NULL,
	"columns" jsonb NOT NULL,
	"version" integer DEFAULT 1,
	"is_template" boolean DEFAULT false,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "lineage" (
	"id" text PRIMARY KEY NOT NULL,
	"source_type" varchar(30) NOT NULL,
	"source_id" text NOT NULL,
	"target_type" varchar(30) NOT NULL,
	"target_id" text NOT NULL,
	"relationship" varchar(30) NOT NULL,
	"metadata" jsonb,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
--> statement-breakpoint
ALTER TABLE "datasets" ADD COLUMN "schema_id" text;