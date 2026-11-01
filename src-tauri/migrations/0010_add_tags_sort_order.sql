-- User-defined display order of tags. NULL means "not placed by the user":
-- such tags sort after every placed tag, by name. Left NULL (no DEFAULT) so
-- that a newly created tag lands at the end without any bookkeeping, and so
-- that a user who never reorders keeps the plain name order.
ALTER TABLE tags ADD COLUMN sort_order INTEGER;

-- Carries the old pinned-tags setting (a JSON array of tag ids) over: pinned
-- tags take 0, 1, 2... in their pinned order, the rest stay NULL and follow by
-- name. The CASE keeps json_each away from anything but a valid array, since
-- json_each raises an error on malformed JSON and this migration must not fail
-- on a hand-edited or corrupt setting. Non-integer elements and ids with no
-- tag are ignored; for a duplicated id the first position wins.
UPDATE tags SET sort_order = (
  SELECT MIN(j.key)
  FROM json_each(
    CASE
      WHEN json_valid((SELECT value FROM settings WHERE key = 'tag_bar_pinned_tags'))
       AND json_type((SELECT value FROM settings WHERE key = 'tag_bar_pinned_tags')) = 'array'
      THEN (SELECT value FROM settings WHERE key = 'tag_bar_pinned_tags')
      ELSE '[]'
    END
  ) AS j
  WHERE j.type = 'integer' AND j.value = tags.id
);

DELETE FROM settings WHERE key = 'tag_bar_pinned_tags';
