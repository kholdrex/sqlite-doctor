-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
ATTACH ':memory:' AS aux;
CREATE TABLE aux.accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO aux.accounts VALUES(1, 'a');
CREATE TABLE aux.orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO aux.orders VALUES(1);
CREATE TABLE aux.__new_accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO aux.__new_accounts SELECT id, name FROM aux.accounts;
SELECT 'before=' || count(*) FROM aux.orders;
DROP TABLE accounts;
PRAGMA foreign_keys=OFF;
DROP TABLE IF EXISTS aux.accounts;
ALTER TABLE aux.__new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM aux.orders;
