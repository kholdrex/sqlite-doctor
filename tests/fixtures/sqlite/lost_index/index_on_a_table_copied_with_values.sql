-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
INSERT INTO users VALUES(1, 'a');
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT NOT NULL);
INSERT INTO __new_users VALUES((SELECT id FROM users), (SELECT email FROM users));
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
