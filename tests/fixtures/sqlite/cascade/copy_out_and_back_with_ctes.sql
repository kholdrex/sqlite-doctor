-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
BEGIN;
CREATE TEMPORARY TABLE aaccounts(id INTEGER, name TEXT);
WITH src AS (SELECT id, name FROM accounts) INSERT INTO aaccounts SELECT id, name FROM src;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT NOT NULL);
WITH held AS (SELECT id, name FROM aaccounts) INSERT INTO accounts SELECT id, name FROM held;
DROP TABLE aaccounts;
SELECT 'after=' || count(*) FROM orders;
COMMIT;
