CREATE TABLE `children` (
	`id` integer PRIMARY KEY NOT NULL,
	`parent_id` integer NOT NULL,
	FOREIGN KEY (`parent_id`) REFERENCES `parents`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE TABLE `parents` (
	`id` integer PRIMARY KEY NOT NULL,
	`name` text
);
