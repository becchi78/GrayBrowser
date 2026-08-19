-- Receiving bin for the .wb kana/roma import. NULL for every row until that
-- import runs; gb_core::search's substring matching must treat NULL as
-- "doesn't match" (never as "matches everything" or crash), never as
-- unchanged-value semantics.
ALTER TABLE videos ADD COLUMN kana TEXT;
ALTER TABLE videos ADD COLUMN roma TEXT;
