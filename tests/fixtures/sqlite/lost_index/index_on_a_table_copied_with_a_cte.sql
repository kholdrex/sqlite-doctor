-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT NOT NULL);
WITH src AS (SELECT id, email FROM users) INSERT INTO __new_users SELECT id, email FROM src;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
