-- sqlite3 prints: before=1 after=0
ATTACH ':memory:' AS aux;
PRAGMA foreign_keys=ON;
CREATE TABLE orders(account_id REFERENCES accounts ON DELETE CASCADE);
CREATE TABLE aux.accounts(id INTEGER PRIMARY KEY, name TEXT);
ALTER TABLE accounts RENAME TO archive;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
INSERT INTO orders VALUES(1);
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO __new_accounts SELECT id, name FROM accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM orders;
