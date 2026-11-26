-- sqlite3 prints: index=0
CREATE TABLE old(email TEXT);
CREATE INDEX idx_users_email ON old(email);
CREATE TEMP TABLE old(email TEXT);
ALTER TABLE old RENAME TO scratch;
DROP TABLE main.old;
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
