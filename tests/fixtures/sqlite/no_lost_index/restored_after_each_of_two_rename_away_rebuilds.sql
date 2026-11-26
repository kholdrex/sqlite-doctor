-- sqlite3 prints: index=1
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
ALTER TABLE users RENAME TO users_old;
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO users SELECT id, email FROM users_old;
DROP TABLE users_old;
CREATE INDEX idx_users_email ON users(email);
ALTER TABLE users RENAME TO users_old;
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO users SELECT id, email FROM users_old;
DROP TABLE users_old;
CREATE INDEX idx_users_email ON users(email);
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
