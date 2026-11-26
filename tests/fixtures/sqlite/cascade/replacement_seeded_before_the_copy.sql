-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
CREATE TABLE defaults(id INTEGER, name TEXT);
INSERT INTO defaults VALUES(2, 'b');
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY, name TEXT);
REPLACE INTO __new_accounts SELECT id, name FROM defaults;
INSERT INTO __new_accounts SELECT id, name FROM accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM orders;
