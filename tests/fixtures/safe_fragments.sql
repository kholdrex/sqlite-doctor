-- Similar fragments must not be promoted to findings without full scope/sequence.
PRAGMA foreign_keys = OFF;
CREATE TABLE __new_drafts (id integer PRIMARY KEY);
INSERT INTO __new_drafts SELECT id FROM drafts;
-- Missing DROP and RENAME.
BEGIN;
SELECT 'PRAGMA foreign_keys=OFF; DROP TABLE drafts;';
COMMIT;
