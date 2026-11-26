-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
CREATE TABLE labels(id INTEGER, label TEXT);
CREATE TEMPORARY TABLE aaccounts(id INTEGER, name TEXT);
INSERT INTO aaccounts SELECT id, (SELECT label FROM labels WHERE labels.id = accounts.id) FROM accounts;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE accounts;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts SELECT id, coalesce(name, (SELECT label FROM labels WHERE labels.id = aaccounts.id)) FROM aaccounts;
DROP TABLE aaccounts;
SELECT 'after=' || count(*) FROM orders;
