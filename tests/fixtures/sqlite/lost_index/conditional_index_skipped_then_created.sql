-- sqlite3 prints: index=0
CREATE TABLE people(id INTEGER PRIMARY KEY);
CREATE INDEX idx_users_email ON people(id);
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
DROP TABLE people;
CREATE INDEX IF NOT EXISTS idx_users_email ON users(id);
CREATE TABLE __new_users(id INTEGER PRIMARY KEY);
INSERT INTO __new_users SELECT id FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
