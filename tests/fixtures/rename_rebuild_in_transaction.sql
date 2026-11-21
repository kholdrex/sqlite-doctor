-- Create, copy, drop, rename inside a transaction that sets the pragma too late.
BEGIN TRANSACTION;
PRAGMA foreign_keys = OFF;

CREATE TABLE "__new_accounts" (
  "id" integer PRIMARY KEY,
  "name" text NOT NULL
);
INSERT INTO "__new_accounts" ("id", "name")
  SELECT "id", "name" FROM "accounts";
DROP TABLE "accounts";
ALTER TABLE "__new_accounts" RENAME TO "accounts";
COMMIT;
