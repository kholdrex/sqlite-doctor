-- sqlite3 prints: before=1 after=1
PRAGMA foreign_keys=ON;
CREATE TABLE tenants(id INTEGER PRIMARY KEY);
INSERT INTO tenants VALUES(1);
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES tenants(id) ON DELETE CASCADE REFERENCES accounts(id));
INSERT INTO orders VALUES(1);
BEGIN;
PRAGMA defer_foreign_keys=ON;
CREATE TEMPORARY TABLE aaccounts(id INTEGER, name TEXT);
INSERT INTO aaccounts SELECT id, name FROM accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts SELECT id, name FROM aaccounts;
DROP TABLE aaccounts;
SELECT 'after=' || count(*) FROM orders;
COMMIT;
