-- sqlite3 prints: before=1 after=1
PRAGMA foreign_keys=ON;
CREATE TABLE clients(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO clients VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES clients(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
ALTER TABLE clients RENAME TO accounts;
ALTER TABLE accounts RENAME TO customers;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(2, 'b');
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO __new_accounts SELECT id, name FROM accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM orders;
