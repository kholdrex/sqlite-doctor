-- sqlite3 prints: trigger=0
CREATE TEMP VIEW staging AS SELECT 1 AS id;
CREATE TRIGGER audit INSTEAD OF INSERT ON staging BEGIN SELECT 1; END;
CREATE TABLE users(id INTEGER PRIMARY KEY);
CREATE TRIGGER IF NOT EXISTS audit AFTER INSERT ON users BEGIN SELECT 1; END;
CREATE TABLE __new_users(id INTEGER PRIMARY KEY);
INSERT INTO __new_users SELECT id FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'trigger=' || count(*) FROM main.sqlite_schema WHERE name = 'audit';
