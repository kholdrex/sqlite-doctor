-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=OFF;
PRAGMA legacy_alter_table=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
ALTER TABLE accounts RENAME TO accounts_old;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
INSERT INTO orders VALUES(1);
PRAGMA foreign_keys=ON;
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO __new_accounts SELECT id, name FROM accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM orders;
