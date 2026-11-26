-- sqlite3 prints: trigger=1
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE VIEW incoming_users AS SELECT id FROM users;
CREATE TRIGGER audit INSTEAD OF INSERT ON incoming_users BEGIN SELECT 1; END;
DROP VIEW incoming_users;
CREATE TRIGGER audit AFTER INSERT ON users BEGIN SELECT 1; END;
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
CREATE TRIGGER IF NOT EXISTS audit AFTER INSERT ON users BEGIN SELECT 1; END;
SELECT 'trigger=' || count(*) FROM main.sqlite_schema WHERE name = 'audit';
