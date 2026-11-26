-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX main.idx_users_email ON users(email);
CREATE TEMP TABLE users(id INTEGER, email TEXT);
BEGIN;
DROP TABLE temp.users;
ROLLBACK;
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_users SELECT id, email FROM main.users;
DROP TABLE main.users;
ALTER TABLE __new_users RENAME TO users;
CREATE INDEX idx_users_email ON users(email);
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
