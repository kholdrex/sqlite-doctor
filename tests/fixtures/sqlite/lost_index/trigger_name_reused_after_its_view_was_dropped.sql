-- sqlite3 prints: trigger=0
CREATE VIEW incoming_users AS SELECT 1 AS id;
CREATE TRIGGER audit INSTEAD OF INSERT ON incoming_users BEGIN SELECT 1; END;
DROP VIEW incoming_users;
CREATE TABLE users(id INTEGER PRIMARY KEY);
CREATE TRIGGER IF NOT EXISTS audit AFTER INSERT ON users BEGIN SELECT 1; END;
CREATE TABLE __new_users(id INTEGER PRIMARY KEY);
INSERT INTO __new_users SELECT id FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'trigger=' || count(*) FROM main.sqlite_schema WHERE name = 'audit';
