-- sqlite3 prints: before=1 after=1
PRAGMA foreign_keys=ON;
CREATE TABLE clients(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO clients VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES clients(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
ALTER TABLE clients RENAME TO accounts;
CREATE TEMP TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
CREATE TEMP TABLE __new_accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO temp.__new_accounts SELECT * FROM temp.accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE temp.accounts;
ALTER TABLE temp.__new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM orders;
