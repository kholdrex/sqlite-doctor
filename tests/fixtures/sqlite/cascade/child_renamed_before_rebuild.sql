-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
ALTER TABLE orders RENAME TO purchases;
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO __new_accounts SELECT id, name FROM accounts;
SELECT 'before=' || count(*) FROM purchases;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM purchases;
