-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX main.idx_users_email ON users(email);
CREATE TEMP TABLE users(id INTEGER, email TEXT);
ALTER TABLE users RENAME TO people;
CREATE TEMP TABLE scratch(id INTEGER);
INSERT INTO scratch SELECT id FROM temp.people;
DROP TABLE temp.people;
ALTER TABLE temp.scratch RENAME TO people;
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_users SELECT id, email FROM main.users;
DROP TABLE main.users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
