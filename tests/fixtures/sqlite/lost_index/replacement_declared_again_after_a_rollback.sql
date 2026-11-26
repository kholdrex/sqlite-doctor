-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
CREATE TABLE held(id INTEGER, email TEXT);
INSERT INTO held SELECT id, email FROM __new_users;
SAVEPOINT s;
DROP TABLE __new_users;
ROLLBACK;
CREATE TABLE IF NOT EXISTS __new_users(id INTEGER PRIMARY KEY, email TEXT);
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
