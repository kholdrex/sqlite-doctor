-- sqlite3 prints: trigger=0
CREATE TABLE users(id INTEGER PRIMARY KEY);
CREATE TRIGGER temp.audit AFTER INSERT ON users BEGIN SELECT 1; END;
DROP TABLE IF EXISTS temp.users;
CREATE TABLE __new_users(id INTEGER PRIMARY KEY);
INSERT INTO __new_users SELECT id FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'trigger=' || count(*) FROM temp.sqlite_schema WHERE name = 'audit';
