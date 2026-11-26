-- sqlite3 prints: trigger=0
CREATE TABLE users(id INTEGER PRIMARY KEY);
CREATE TEMP TRIGGER audit AFTER INSERT ON main.users BEGIN SELECT 1; END;
CREATE TEMP TABLE users(id INTEGER);
DROP TABLE temp.users;
CREATE TABLE __new_users(id INTEGER PRIMARY KEY);
INSERT INTO __new_users SELECT id FROM main.users;
DROP TABLE main.users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'trigger=' || count(*) FROM temp.sqlite_schema WHERE name = 'audit';
