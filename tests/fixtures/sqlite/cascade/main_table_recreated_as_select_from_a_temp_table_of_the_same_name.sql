-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
CREATE TEMP TABLE accounts AS SELECT id, name FROM main.accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE main.accounts;
CREATE TABLE accounts AS SELECT id, name FROM accounts;
SELECT 'after=' || count(*) FROM orders;
