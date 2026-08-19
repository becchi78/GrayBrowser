-- Adds per-video failure-attempt counters for the thumbnail and
-- metadata background generation pipelines. Both workers originally shipped
-- with no upper bound on retrying a permanently-failing file (e.g. a
-- corrupt/unsupported video) -- every scan/poll cycle would re-attempt it
-- forever. `gb_core::retry::MAX_GENERATION_ATTEMPTS`/
-- `is_eligible_for_automatic_retry` define the pure classification logic;
-- these two columns are the persisted state it classifies. `DEFAULT 0`
-- makes every pre-existing row (and every row inserted before either worker
-- ever runs) start at "no attempts yet" without a backfill step.
ALTER TABLE videos ADD COLUMN thumbnail_attempts INTEGER NOT NULL DEFAULT 0;
ALTER TABLE videos ADD COLUMN metadata_attempts INTEGER NOT NULL DEFAULT 0;
