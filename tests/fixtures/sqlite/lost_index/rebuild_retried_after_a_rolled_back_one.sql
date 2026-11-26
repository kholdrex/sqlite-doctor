-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE first_try(id INTEGER PRIMARY KEY);
INSERT INTO first_try SELECT id FROM users;
CREATE TABLE second_try(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO second_try SELECT id, email FROM users;
SAVEPOINT s;
DROP TABLE users;
ALTER TABLE first_try RENAME TO users;
ROLLBACK;
DROP TABLE users;
ALTER TABLE second_try RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
