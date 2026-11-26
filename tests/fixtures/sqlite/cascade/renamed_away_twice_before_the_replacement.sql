-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
ALTER TABLE accounts RENAME TO accounts_old;
ALTER TABLE accounts_old RENAME TO accounts_backup;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT NOT NULL);
INSERT INTO accounts SELECT id, name FROM accounts_backup;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts_backup;
SELECT 'after=' || count(*) FROM orders;
