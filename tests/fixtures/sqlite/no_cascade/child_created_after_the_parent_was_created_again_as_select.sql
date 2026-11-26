-- sqlite3 prints: before=1 after=1
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
ALTER TABLE accounts RENAME TO accounts_old;
CREATE TABLE accounts AS SELECT id, name FROM accounts_old;
CREATE UNIQUE INDEX accounts_id ON accounts(id);
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts_old;
SELECT 'after=' || count(*) FROM orders;
