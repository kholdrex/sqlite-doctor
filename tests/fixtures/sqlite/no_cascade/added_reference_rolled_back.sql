-- sqlite3 prints: before=1 after=1
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(id INTEGER);
INSERT INTO orders VALUES(1);
SAVEPOINT add_reference;
ALTER TABLE orders ADD COLUMN account_id REFERENCES accounts(id) ON DELETE CASCADE;
ROLLBACK TO add_reference;
RELEASE add_reference;
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO __new_accounts SELECT id, name FROM accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM orders;
