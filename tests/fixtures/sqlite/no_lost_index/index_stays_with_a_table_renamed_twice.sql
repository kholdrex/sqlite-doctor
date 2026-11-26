-- sqlite3 prints: index=1
CREATE TABLE clients(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON clients(email);
ALTER TABLE clients RENAME TO users;
ALTER TABLE users RENAME TO customers;
CREATE TABLE users(id INTEGER PRIMARY KEY, email TEXT);
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
