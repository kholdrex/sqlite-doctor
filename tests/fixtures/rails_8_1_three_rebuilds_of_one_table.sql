-- Captured with contrib/rails/sqlite_doctor_capture.rb on ActiveRecord 8.1.4 from two migrations:
-- change_column_null on parents, then remove_column and change_column_default on parents.
-- Observed on the copy: rows in children 2 -> 0

-- Baseline schema
CREATE TABLE tenants (id INTEGER PRIMARY KEY);
CREATE TABLE parents (id INTEGER PRIMARY KEY, tenant_id INTEGER, name TEXT, code TEXT);
CREATE TABLE children (id INTEGER PRIMARY KEY, parent_id INTEGER NOT NULL REFERENCES parents(id) ON DELETE CASCADE);
CREATE INDEX idx_parents_name ON parents(name);
CREATE INDEX idx_parents_code ON parents(code);
CREATE INDEX idx_parents_tenant ON parents(tenant_id);

-- Connection state before migrating
PRAGMA foreign_keys = 1;

-- Executed statements
SELECT name FROM pragma_table_list WHERE schema <> 'temp' AND name NOT IN ('sqlite_sequence', 'sqlite_schema') AND name = 'schema_migrations' AND type IN ('table');
CREATE TABLE "schema_migrations" ("version" varchar NOT NULL PRIMARY KEY);
SELECT name FROM pragma_table_list WHERE schema <> 'temp' AND name NOT IN ('sqlite_sequence', 'sqlite_schema') AND name = 'ar_internal_metadata' AND type IN ('table');
CREATE TABLE "ar_internal_metadata" ("key" varchar NOT NULL PRIMARY KEY, "value" varchar, "created_at" datetime(6) NOT NULL, "updated_at" datetime(6) NOT NULL);
SELECT * FROM "ar_internal_metadata" WHERE "ar_internal_metadata"."key" = ? ORDER BY "ar_internal_metadata"."key" ASC LIMIT 1;
INSERT INTO "ar_internal_metadata" ("key", "value", "created_at", "updated_at") VALUES ('environment', 'default_env', '2026-11-26 06:13:43.981646', '2026-11-26 06:13:43.981648') RETURNING "key";
SELECT "schema_migrations"."version" FROM "schema_migrations" ORDER BY "schema_migrations"."version" ASC;
BEGIN immediate TRANSACTION;
PRAGMA foreign_key_list('parents');
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
            SELECT sql
            FROM sqlite_master
            WHERE name = 'parents' AND type = 'table'
            UNION ALL
            SELECT sql
            FROM sqlite_temp_master
            WHERE name = 'parents' AND type = 'table';
PRAGMA foreign_keys;
PRAGMA defer_foreign_keys;
PRAGMA defer_foreign_keys = ON;
PRAGMA foreign_keys = OFF;
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
CREATE TEMPORARY TABLE "aparents" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL, "tenant_id" integer, "name" text, "code" text);
PRAGMA index_list("parents");
SELECT sql
FROM sqlite_master
WHERE name = 'idx_parents_tenant' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'idx_parents_tenant' AND type = 'index';
PRAGMA index_info('idx_parents_tenant');
SELECT sql
FROM sqlite_master
WHERE name = 'idx_parents_code' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'idx_parents_code' AND type = 'index';
PRAGMA index_info('idx_parents_code');
SELECT sql
FROM sqlite_master
WHERE name = 'idx_parents_name' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'idx_parents_name' AND type = 'index';
PRAGMA index_info('idx_parents_name');
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
CREATE INDEX "tidx_aparents_tenant" ON "aparents" ("tenant_id");
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
CREATE INDEX "tidx_aparents_code" ON "aparents" ("code");
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
CREATE INDEX "tidx_aparents_name" ON "aparents" ("name");
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
INSERT INTO "aparents" ("id","tenant_id","name","code")
                     SELECT "id","tenant_id","name","code" FROM "parents";
DROP TABLE "parents";
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
CREATE TABLE "parents" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL, "tenant_id" integer, "name" text NOT NULL, "code" text);
PRAGMA index_list("aparents");
SELECT sql
FROM sqlite_master
WHERE name = 'tidx_aparents_name' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'tidx_aparents_name' AND type = 'index';
PRAGMA index_info('tidx_aparents_name');
SELECT sql
FROM sqlite_master
WHERE name = 'tidx_aparents_code' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'tidx_aparents_code' AND type = 'index';
PRAGMA index_info('tidx_aparents_code');
SELECT sql
FROM sqlite_master
WHERE name = 'tidx_aparents_tenant' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'tidx_aparents_tenant' AND type = 'index';
PRAGMA index_info('tidx_aparents_tenant');
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
CREATE INDEX "idx_parents_name" ON "parents" ("name");
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
CREATE INDEX "idx_parents_code" ON "parents" ("code");
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
CREATE INDEX "idx_parents_tenant" ON "parents" ("tenant_id");
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
INSERT INTO "parents" ("id","tenant_id","name","code")
                     SELECT "id","tenant_id","name","code" FROM "aparents";
DROP TABLE "aparents";
PRAGMA defer_foreign_keys = 0;
PRAGMA foreign_keys = 1;
INSERT INTO "schema_migrations" ("version") VALUES ('20260101000001') RETURNING "version";
COMMIT TRANSACTION;
BEGIN immediate TRANSACTION;
PRAGMA foreign_key_list('parents');
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
            SELECT sql
            FROM sqlite_master
            WHERE name = 'parents' AND type = 'table'
            UNION ALL
            SELECT sql
            FROM sqlite_temp_master
            WHERE name = 'parents' AND type = 'table';
PRAGMA foreign_keys;
PRAGMA defer_foreign_keys;
PRAGMA defer_foreign_keys = ON;
PRAGMA foreign_keys = OFF;
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
CREATE TEMPORARY TABLE "aparents" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL, "tenant_id" integer, "name" text NOT NULL, "code" text);
PRAGMA index_list("parents");
SELECT sql
FROM sqlite_master
WHERE name = 'idx_parents_tenant' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'idx_parents_tenant' AND type = 'index';
PRAGMA index_info('idx_parents_tenant');
SELECT sql
FROM sqlite_master
WHERE name = 'idx_parents_code' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'idx_parents_code' AND type = 'index';
PRAGMA index_info('idx_parents_code');
SELECT sql
FROM sqlite_master
WHERE name = 'idx_parents_name' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'idx_parents_name' AND type = 'index';
PRAGMA index_info('idx_parents_name');
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
CREATE INDEX "tidx_aparents_tenant" ON "aparents" ("tenant_id");
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
CREATE INDEX "tidx_aparents_code" ON "aparents" ("code");
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
CREATE INDEX "tidx_aparents_name" ON "aparents" ("name");
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
INSERT INTO "aparents" ("id","tenant_id","name","code")
                     SELECT "id","tenant_id","name","code" FROM "parents";
DROP TABLE "parents";
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
CREATE TABLE "parents" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL, "tenant_id" integer, "name" text NOT NULL);
PRAGMA index_list("aparents");
SELECT sql
FROM sqlite_master
WHERE name = 'tidx_aparents_name' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'tidx_aparents_name' AND type = 'index';
PRAGMA index_info('tidx_aparents_name');
SELECT sql
FROM sqlite_master
WHERE name = 'tidx_aparents_code' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'tidx_aparents_code' AND type = 'index';
PRAGMA index_info('tidx_aparents_code');
SELECT sql
FROM sqlite_master
WHERE name = 'tidx_aparents_tenant' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'tidx_aparents_tenant' AND type = 'index';
PRAGMA index_info('tidx_aparents_tenant');
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
CREATE INDEX "idx_parents_name" ON "parents" ("name");
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
CREATE INDEX "idx_parents_tenant" ON "parents" ("tenant_id");
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
INSERT INTO "parents" ("id","tenant_id","name")
                     SELECT "id","tenant_id","name" FROM "aparents";
DROP TABLE "aparents";
PRAGMA defer_foreign_keys = 0;
PRAGMA foreign_keys = 1;
PRAGMA foreign_key_list('parents');
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
            SELECT sql
            FROM sqlite_master
            WHERE name = 'parents' AND type = 'table'
            UNION ALL
            SELECT sql
            FROM sqlite_temp_master
            WHERE name = 'parents' AND type = 'table';
PRAGMA foreign_keys;
PRAGMA defer_foreign_keys;
PRAGMA defer_foreign_keys = ON;
PRAGMA foreign_keys = OFF;
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
CREATE TEMPORARY TABLE "aparents" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL, "tenant_id" integer, "name" text NOT NULL);
PRAGMA index_list("parents");
SELECT sql
FROM sqlite_master
WHERE name = 'idx_parents_tenant' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'idx_parents_tenant' AND type = 'index';
PRAGMA index_info('idx_parents_tenant');
SELECT sql
FROM sqlite_master
WHERE name = 'idx_parents_name' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'idx_parents_name' AND type = 'index';
PRAGMA index_info('idx_parents_name');
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
CREATE INDEX "tidx_aparents_tenant" ON "aparents" ("tenant_id");
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
CREATE INDEX "tidx_aparents_name" ON "aparents" ("name");
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
INSERT INTO "aparents" ("id","tenant_id","name")
                     SELECT "id","tenant_id","name" FROM "parents";
DROP TABLE "parents";
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
CREATE TABLE "parents" ("id" integer PRIMARY KEY AUTOINCREMENT NOT NULL, "tenant_id" integer, "name" text DEFAULT 'x' NOT NULL);
PRAGMA index_list("aparents");
SELECT sql
FROM sqlite_master
WHERE name = 'tidx_aparents_name' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'tidx_aparents_name' AND type = 'index';
PRAGMA index_info('tidx_aparents_name');
SELECT sql
FROM sqlite_master
WHERE name = 'tidx_aparents_tenant' AND type = 'index'
UNION ALL
SELECT sql
FROM sqlite_temp_master
WHERE name = 'tidx_aparents_tenant' AND type = 'index';
PRAGMA index_info('tidx_aparents_tenant');
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
CREATE INDEX "idx_parents_name" ON "parents" ("name");
PRAGMA table_xinfo("parents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'parents';
CREATE INDEX "idx_parents_tenant" ON "parents" ("tenant_id");
PRAGMA table_xinfo("aparents");
SELECT sql FROM
  (SELECT * FROM sqlite_master UNION ALL
   SELECT * FROM sqlite_temp_master)
WHERE type = 'table' AND name = 'aparents';
INSERT INTO "parents" ("id","tenant_id","name")
                     SELECT "id","tenant_id","name" FROM "aparents";
DROP TABLE "aparents";
PRAGMA defer_foreign_keys = 0;
PRAGMA foreign_keys = 1;
INSERT INTO "schema_migrations" ("version") VALUES ('20260101000002') RETURNING "version";
COMMIT TRANSACTION;
