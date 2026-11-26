-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE TEMP TABLE users(id INTEGER, email TEXT);
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
REPLACE INTO __new_users SELECT id, email FROM temp.users;
BEGIN;
DROP TABLE temp.users;
ROLLBACK;
DROP TABLE users;
CREATE INDEX idx_users_email ON users(email);
INSERT INTO __new_users SELECT id, email FROM main.users;
DROP TABLE main.users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
