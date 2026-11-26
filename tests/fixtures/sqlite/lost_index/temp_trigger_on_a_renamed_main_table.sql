-- sqlite3 prints: trigger=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE TEMP TRIGGER users_audit AFTER INSERT ON main.users BEGIN SELECT 1; END;
ALTER TABLE users RENAME TO people;
ALTER TABLE main.people RENAME TO users;
CREATE TABLE people(id INTEGER PRIMARY KEY, email TEXT);
CREATE TEMP TRIGGER IF NOT EXISTS users_audit AFTER INSERT ON people BEGIN SELECT 1; END;
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_users SELECT id, email FROM main.users;
DROP TABLE main.users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'trigger=' || count(*) FROM temp.sqlite_schema WHERE name = 'users_audit';
