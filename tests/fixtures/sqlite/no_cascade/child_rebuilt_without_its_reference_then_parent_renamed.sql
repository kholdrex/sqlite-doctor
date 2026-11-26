-- sqlite3 prints: before=1 after=1
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
CREATE TABLE __new_orders(account_id INTEGER);
INSERT INTO __new_orders SELECT account_id FROM orders;
DROP TABLE orders;
ALTER TABLE __new_orders RENAME TO orders;
ALTER TABLE accounts RENAME TO customers;
CREATE TABLE __new_customers(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO __new_customers SELECT id, name FROM customers;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE customers;
ALTER TABLE __new_customers RENAME TO customers;
SELECT 'after=' || count(*) FROM orders;
