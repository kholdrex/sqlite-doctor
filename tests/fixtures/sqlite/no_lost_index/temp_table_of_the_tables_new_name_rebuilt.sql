-- sqlite3 prints: index=1
CREATE TABLE clients(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON clients(email);
ALTER TABLE clients RENAME TO users;
CREATE TEMP TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE TEMP TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO temp.__new_users SELECT * FROM temp.users;
DROP TABLE temp.users;
ALTER TABLE temp.__new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
