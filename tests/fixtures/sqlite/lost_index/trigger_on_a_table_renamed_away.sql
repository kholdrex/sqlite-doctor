-- sqlite3 prints: trigger=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE TABLE audit(email TEXT);
CREATE TRIGGER users_audit AFTER INSERT ON users BEGIN INSERT INTO audit VALUES(new.email); END;
ALTER TABLE users RENAME TO users_old;
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT NOT NULL);
INSERT INTO users SELECT id, email FROM users_old;
DROP TABLE users_old;
SELECT 'trigger=' || count(*) FROM main.sqlite_schema WHERE name = 'users_audit';
