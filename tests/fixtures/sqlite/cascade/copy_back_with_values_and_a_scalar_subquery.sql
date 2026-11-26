-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
CREATE TEMP TABLE held(id INTEGER, name TEXT);
INSERT INTO held(id, name) VALUES((SELECT id FROM accounts), 'a');
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT NOT NULL);
REPLACE INTO accounts(id, name) VALUES((SELECT id FROM held), (SELECT name FROM held));
SELECT 'after=' || count(*) FROM orders;
