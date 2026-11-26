-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE held_users(id INTEGER, email TEXT);
INSERT INTO held_users SELECT * FROM users;
SAVEPOINT s;
DROP TABLE 'users';
CREATE TABLE users(id INTEGER PRIMARY KEY);
ROLLBACK;
DROP TABLE users;
CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO users SELECT * FROM held_users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
