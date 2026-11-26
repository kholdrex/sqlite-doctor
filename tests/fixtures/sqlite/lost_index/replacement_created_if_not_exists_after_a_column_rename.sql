-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
ALTER TABLE users RENAME COLUMN email TO mail;
CREATE INDEX idx_users_email ON users(mail);
ALTER TABLE users RENAME TO users_old;
CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY, mail TEXT);
INSERT INTO users SELECT id, mail FROM users_old;
DROP TABLE users_old;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
