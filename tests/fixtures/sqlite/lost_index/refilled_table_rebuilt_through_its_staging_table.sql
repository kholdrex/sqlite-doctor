-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE archive(id INTEGER, email TEXT);
CREATE TABLE stage(id INTEGER, email TEXT);
INSERT INTO users SELECT id, email FROM archive;
INSERT OR REPLACE INTO users SELECT id, email FROM stage;
DROP TABLE stage;
CREATE TABLE stage(id INTEGER, email TEXT);
INSERT INTO stage SELECT id, email FROM users;
DROP TABLE users;
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO users SELECT id, email FROM stage;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
