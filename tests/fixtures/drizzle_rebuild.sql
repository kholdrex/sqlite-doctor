-- Drizzle-inspired generated migration. The pragma is outside transaction scope.
PRAGMA foreign_keys=OFF;
BEGIN;
CREATE TABLE `__new_users` (
  `id` integer PRIMARY KEY,
  `email` text NOT NULL
);
INSERT INTO `__new_users` (`id`, `email`) SELECT `id`, `email` FROM `users`;
DROP TABLE `users`;
ALTER TABLE `__new_users` RENAME TO `users`;
COMMIT;
PRAGMA foreign_keys=ON;
