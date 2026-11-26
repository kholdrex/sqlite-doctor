-- sqlite3 prints: index=0
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON users(email);
ALTER TABLE users RENAME TO people;
CREATE TABLE __new_people(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_people SELECT id, email FROM people;
DROP TABLE people;
ALTER TABLE __new_people RENAME TO people;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
