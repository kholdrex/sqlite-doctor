-- sqlite3 prints: before=1 after=0
PRAGMA foreign_keys=ON;
CREATE TEMP TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
CREATE TEMP TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
INSERT INTO orders VALUES(1);
CREATE TEMP VIEW one AS SELECT 1;
DROP TABLE IF EXISTS main.orders;
CREATE TEMP TABLE __new_accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO __new_accounts SELECT id, name FROM temp.accounts;
SELECT 'before=' || count(*) FROM temp.orders;
DROP TABLE temp.accounts;
ALTER TABLE temp.__new_accounts RENAME TO accounts;
SELECT 'after=' || count(*) FROM temp.orders;
