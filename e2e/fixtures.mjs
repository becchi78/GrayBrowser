// Test-data setup shared by the E2E specs. Fixture "video" files are
// generated at test-run time into a fresh OS temp folder -- never committed
// to the repo -- and must be non-empty, since a 0-byte file is treated as
// corrupt and skipped from DB registration entirely, which would leave
// nothing for these specs to find.

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { appDbPath } from "./session.mjs";

export function createFixtureFolder(fileNames) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "gb-e2e-"));
  for (const name of fileNames) {
    // Content is irrelevant (not a real, decodable video) -- these specs
    // only exercise registration/tagging/search, never thumbnail or
    // metadata extraction, which fail harmlessly against fake bytes.
    fs.writeFileSync(path.join(dir, name), `not a real video -- gb e2e fixture (${name})`);
  }
  return dir;
}

// The app's folder picker opens a native OS dialog (tauri-plugin-dialog),
// which WebDriver cannot drive -- it only automates the WebView2-rendered
// page, not native Win32 UI. So instead of clicking "フォルダを追加", this
// writes directly to the `settings` table the picker would have written to,
// the same workaround this project's own db::queries::set_watch_folders
// callers use programmatically. Requires app.db to already exist (i.e. the
// app must have been launched at least once already, so db::init has run
// and created the settings table) -- WAL mode allows this external
// single-statement write to interleave safely with the running app's own
// connections.
export function seedWatchFolder(folderPath) {
  const db = new DatabaseSync(appDbPath());
  try {
    const stmt = db.prepare(
      "INSERT INTO settings (key, value) VALUES ('watch_folders', ?) " +
        "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    );
    stmt.run(JSON.stringify([folderPath]));
  } finally {
    db.close();
  }
}

export function cleanupFixtureFolder(dir) {
  fs.rmSync(dir, { recursive: true, force: true });
}

// tags/tag_bar_pinned_tags seeding for tag-bar.e2e.mjs. Same "write directly
// to the row the real UI would otherwise produce" shape as seedWatchFolder
// above -- driving TagBarEditDialog's ↑/↓/✕/+ 追加 buttons through WebDriver
// would work too, but this is faster/more deterministic, and this project's
// e2e history already prefers a direct DB write wherever the schema is
// simple enough for one (see seedWatchFolder's own comment).
export function seedTags(names) {
  const db = new DatabaseSync(appDbPath());
  try {
    return names.map((name) => {
      db.prepare(
        "INSERT INTO tags (name) VALUES (?) ON CONFLICT(name) DO UPDATE SET name = excluded.name",
      ).run(name);
      return db.prepare("SELECT id FROM tags WHERE name = ?").get(name).id;
    });
  } finally {
    db.close();
  }
}

// Reads the currently-persisted pinned list so a test can restore it in its
// own `finally` block -- `tag_bar_pinned_tags` is a single global `settings`
// row (unlike a per-video tag assignment), so leaving a test's seeded list
// in place would leak into every e2e spec that runs afterward against this
// same, persistent app.db.
export function getTagBarPinnedTagIds() {
  const db = new DatabaseSync(appDbPath());
  try {
    const row = db.prepare("SELECT value FROM settings WHERE key = 'tag_bar_pinned_tags'").get();
    return row ? JSON.parse(row.value) : [];
  } finally {
    db.close();
  }
}

export function seedTagBarPinnedTagIds(tagIds) {
  const db = new DatabaseSync(appDbPath());
  try {
    db.prepare(
      "INSERT INTO settings (key, value) VALUES ('tag_bar_pinned_tags', ?) " +
        "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    ).run(JSON.stringify(tagIds));
  } finally {
    db.close();
  }
}
