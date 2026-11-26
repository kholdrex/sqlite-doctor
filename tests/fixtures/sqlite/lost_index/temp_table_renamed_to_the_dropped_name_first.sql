-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_users SELECT * FROM main.users;
CREATE TABLE other(id INTEGER);
CREATE TEMP TABLE work(id INTEGER);
INSERT INTO work SELECT id FROM other;
INSERT INTO work SELECT id FROM main.users;
DROP TABLE main.users;
ALTER TABLE temp.work RENAME TO users;
ALTER TABLE main.__new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
