-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
ATTACH ':memory:' AS aux;
CREATE TABLE aux.accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO aux.accounts VALUES(1, 'a');
CREATE TABLE aux.orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO aux.orders VALUES(1);
ALTER TABLE accounts RENAME TO customers;
CREATE TABLE aux.__new_customers(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO aux.__new_customers SELECT id, name FROM aux.customers;
SELECT 'before=' || count(*) FROM aux.orders;
DROP TABLE aux.customers;
ALTER TABLE aux.__new_customers RENAME TO customers;
SELECT 'after=' || count(*) FROM aux.orders;
