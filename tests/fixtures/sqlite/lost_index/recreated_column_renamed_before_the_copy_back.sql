-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users('email');
CREATE TEMP TABLE held(id INTEGER, email TEXT);
INSERT INTO held SELECT id, email FROM users;
DROP TABLE users;
CREATE TABLE users(id INTEGER PRIMARY KEY, old_email TEXT);
ALTER TABLE users RENAME COLUMN old_email TO email;
INSERT INTO users(id, email) SELECT id, email FROM held;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
