-- sqlite3 prints: before=1 after=1
PRAGMA foreign_keys=OFF;
PRAGMA legacy_alter_table=ON;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(2, 'b');
CREATE TABLE orders(account_id REFERENCES accounts(id) ON DELETE CASCADE);
ALTER TABLE accounts RENAME TO archive;
CREATE TABLE accounts(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO accounts VALUES(1, 'a');
INSERT INTO orders VALUES(1);
PRAGMA foreign_keys=ON;
CREATE TABLE __new_archive(id INTEGER PRIMARY KEY, name TEXT);
INSERT INTO __new_archive SELECT id, name FROM archive;
SELECT 'before=' || count(*) FROM orders;
DROP TABLE archive;
ALTER TABLE __new_archive RENAME TO archive;
SELECT 'after=' || count(*) FROM orders;
