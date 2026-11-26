-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
CREATE TABLE __new_accounts(id INTEGER PRIMARY KEY, name TEXT NOT NULL);
WITH RECURSIVE replace(id) AS MATERIALIZED (SELECT id FROM accounts), named(id, name) AS (SELECT a.id, a.name FROM replace JOIN accounts AS a USING (id))
REPLACE INTO __new_accounts SELECT id, name FROM named;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM orders;
