-- sqlite3 prints: before=1 after=1
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
CREATE TABLE accounts_copy AS SELECT id, name FROM accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts_copy;
SELECT 'after=' || count(*) FROM orders;
