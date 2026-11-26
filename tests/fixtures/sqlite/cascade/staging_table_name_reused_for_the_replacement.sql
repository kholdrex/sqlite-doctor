-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
CREATE TABLE stage(id INTEGER, name TEXT);
INSERT INTO stage VALUES(2, 'b');
REPLACE INTO accounts SELECT id, name FROM stage;
DROP TABLE stage;
CREATE TABLE stage(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO stage SELECT id, name FROM accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
ALTER TABLE stage RENAME TO accounts;
SELECT 'after=' || count(*) FROM orders;
