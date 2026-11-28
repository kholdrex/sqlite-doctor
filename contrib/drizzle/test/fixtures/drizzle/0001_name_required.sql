PRAGMA foreign_keys=OFF;--> statement-breakpoint
CREATE TABLE `__new_parents` (
	`id` integer PRIMARY KEY NOT NULL,
	`name` text NOT NULL
);
--> statement-breakpoint
INSERT INTO `__new_parents`("id", "name") SELECT "id", "name" FROM `parents`;--> statement-breakpoint
DROP TABLE `parents`;--> statement-breakpoint
ALTER TABLE `__new_parents` RENAME TO `parents`;--> statement-breakpoint
PRAGMA foreign_keys=ON;