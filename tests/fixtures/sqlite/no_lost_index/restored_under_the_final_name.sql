-- sqlite3 prints: index=1
CREATE TABLE customers(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON customers(email);
ALTER TABLE customers RENAME TO users;
BEGIN;
CREATE TABLE __new_users(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_users SELECT id, email FROM users;
DROP TABLE users;
ALTER TABLE __new_users RENAME TO users;
ALTER TABLE users RENAME TO accounts;
CREATE INDEX idx_users_email ON accounts(email);
COMMIT;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
