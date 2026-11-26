-- sqlite3 prints: trigger=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE TABLE audit(email TEXT);
CREATE TRIGGER users_audit AFTER INSERT ON users BEGIN INSERT INTO audit VALUES(new.email); END;
CREATE TABLE __new_users AS SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'trigger=' || count(*) FROM main.sqlite_schema WHERE name = 'users_audit';
