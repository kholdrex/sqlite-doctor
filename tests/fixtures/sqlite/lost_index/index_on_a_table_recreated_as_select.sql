-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
CREATE TEMP TABLE held AS SELECT id, email FROM users;
DROP TABLE users;
CREATE TABLE users AS SELECT id, email FROM held;
DROP TABLE held;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
