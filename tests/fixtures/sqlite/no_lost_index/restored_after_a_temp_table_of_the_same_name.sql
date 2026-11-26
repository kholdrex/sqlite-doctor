-- sqlite3 prints: index=1
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
BEGIN;
CREATE TEMPORARY TABLE users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO temp.users SELECT * FROM main.users;
DROP TABLE main.users;
CREATE TABLE main.users(id INTEGER PRIMARY KEY, email TEXT NOT NULL);
INSERT INTO main.users SELECT * FROM temp.users;
DROP TABLE temp.users;
CREATE INDEX idx_users_email ON users(email);
COMMIT;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
