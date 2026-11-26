-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
ALTER TABLE accounts RENAME TO accounts_old;
CREATE TABLE IF NOT EXISTS accounts(id INTEGER PRIMARY KEY, name TEXT NOT NULL);
INSERT INTO accounts SELECT id, name FROM accounts_old;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts_old;
SELECT 'after=' || count(*) FROM orders;
