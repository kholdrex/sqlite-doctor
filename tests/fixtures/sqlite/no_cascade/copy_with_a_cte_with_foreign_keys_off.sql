-- sqlite3 prints: before=1 after=1
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
PRAGMA foreign_keys=OFF;
BEGIN;
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY, name TEXT NOT NULL);
WITH src AS (SELECT id, name FROM accounts) INSERT INTO __new_accounts SELECT id, name FROM src;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM orders;
COMMIT;
PRAGMA foreign_keys=ON;
