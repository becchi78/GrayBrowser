-- Tag groups: a display-only container for tags in the left pane. A group does
-- not change what a tag filters by. sort_order is NULL until the user (or the
-- first layout save) places it; it shares one number line with ungrouped
-- tags.sort_order to decide the top-level order.
--
-- AUTOINCREMENT so a deleted group's id is never handed to a new group: the
-- collapsed/expanded state is stored by group id in settings, and a reused id
-- would make a new group inherit another group's saved state.
CREATE TABLE tag_groups (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL UNIQUE,
  sort_order INTEGER
);

-- NULL = the tag is at the top level, outside any group. No FOREIGN KEY: the
-- app runs with foreign_keys=OFF, so it would not be enforced anyway; the
-- application layer keeps it consistent (deleting a group resets its tags'
-- group_id in the same transaction) and a dangling value is read as top level.
-- Existing rows stay NULL, so existing tag order is unchanged.
ALTER TABLE tags ADD COLUMN group_id INTEGER;
