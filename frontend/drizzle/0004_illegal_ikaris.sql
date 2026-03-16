CREATE TABLE "assets" (
	"id" text PRIMARY KEY NOT NULL,
	"name" text NOT NULL,
	"description" text,
	"folder" text DEFAULT '/',
	"mime_type" text,
	"file_type" varchar(20),
	"file_path" text NOT NULL,
	"file_name" text NOT NULL,
	"size_bytes" bigint DEFAULT 0,
	"tags" text[],
	"metadata" jsonb,
	"dataset_id" text,
	"created_at" timestamp with time zone DEFAULT now() NOT NULL,
	"updated_at" timestamp with time zone DEFAULT now() NOT NULL
);
