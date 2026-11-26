-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=OFF;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY, name TEXT);
CREATE TEMP TABLE accounts(id INTEGER, name TEXT);
INSERT INTO temp.accounts VALUES(2, 'b');
INSERT INTO __new_accounts SELECT id, name FROM temp.accounts;
DROP TABLE temp.accounts;
PRAGMA foreign_keys=ON;
INSERT INTO __new_accounts SELECT id, name FROM main.accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE main.accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM orders;
