-- sqlite3 prints: index=0
PRAGMA foreign_keys=ON;
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_users SELECT id, email FROM main.users;
CREATE TEMP TABLE users(id INTEGER, email TEXT);
INSERT INTO __new_users SELECT id, email FROM temp.users;
DROP TABLE temp.users;
CREATE INDEX idx_users_email ON users(email);
PRAGMA foreign_keys=OFF;
DROP TABLE main.users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
