-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE held(id INTEGER, email TEXT);
INSERT INTO held SELECT * FROM users;
DROP TABLE users;
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
DROP TABLE IF EXISTS temp.users;
CREATE TEMP TABLE users(id INTEGER);
INSERT INTO main.users SELECT * FROM held;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
