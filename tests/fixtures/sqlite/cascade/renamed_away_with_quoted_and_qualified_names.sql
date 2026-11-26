-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
ALTER TABLE main."Accounts" RENAME TO [accounts old];
CREATE TABLE `accounts`(id INTEGER PRIMARY KEY, name TEXT NOT NULL);
INSERT OR REPLACE INTO main.accounts(id, name) SELECT id, name FROM "accounts old";
SELECT 'before=' || count(*) FROM orders;
DROP TABLE IF EXISTS main."ACCOUNTS OLD";
SELECT 'after=' || count(*) FROM orders;
