-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE TEMP TABLE users(id INTEGER, email TEXT);
DROP TABLE temp.users;
CREATE INDEX idx_users_email ON users(email);
DROP INDEX IF EXISTS temp.'idx_users_email';
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
