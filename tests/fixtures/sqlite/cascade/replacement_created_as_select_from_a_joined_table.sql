-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
CREATE TABLE labels(id INTEGER, label TEXT);
INSERT INTO labels VALUES(1, 'x');
CREATE TABLE __new_accounts AS SELECT accounts.id, labels.label AS name FROM labels JOIN accounts USING(id);
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
ALTER TABLE __new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM orders;
