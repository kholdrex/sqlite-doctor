-- sqlite3 prints: index=0
CREATE TABLE temp(id INTEGER PRIMARY KEY, email TEXT);
CREATE INDEX idx_users_email ON temp(email);
DROP TABLE IF EXISTS temp.temp;
CREATE TABLE __new_temp(id INTEGER PRIMARY KEY, email TEXT);
INSERT INTO __new_temp SELECT id, email FROM temp;
DROP TABLE temp;
ALTER TABLE __new_temp RENAME TO temp;
SELECT 'index=' || count(*) FROM main.sqlite_schema WHERE name = 'idx_users_email';
