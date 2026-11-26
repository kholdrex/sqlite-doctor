-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
CREATE TEMP TABLE held_users(id INTEGER, email TEXT);
INSERT INTO held_users SELECT * FROM users;
DROP TABLE users;
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY);
INSERT INTO users SELECT * FROM held_users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
