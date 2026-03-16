CREATE TABLE "data_versions" (
	"id" text PRIMARY KEY NOT NULL,
	"dataset_id" text NOT NULL,
	"version_num" integer DEFAULT 1,
	"file_path" text NOT NULL,
	"size_bytes" bigint DEFAULT 0,
	"rows_affected" integer DEFAULT 0,
	"change_type" varchar(20),
	"change_summary" text,
	"created_by" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL
);
