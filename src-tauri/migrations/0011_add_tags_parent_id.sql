-- Parent of a tag, for a two-level hierarchy. NULL means top-level, which is
-- what every existing tag becomes. Left without a foreign key like every other
-- reference in this schema (foreign_keys is OFF); the application layer keeps
-- it pointing at an existing top-level tag.
ALTER TABLE tags ADD COLUMN parent_id INTEGER;
