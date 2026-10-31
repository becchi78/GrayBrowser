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

// tags/tags.sort_order seeding for tag-bar.e2e.mjs. Same "write directly
// to the row the real UI would otherwise produce" shape as seedWatchFolder
// above -- driving TagBarEditDialog's ↑/↓ buttons through WebDriver
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

// Reads every tag's persisted `sort_order` (NULL = not yet ordered) so a test
// can restore it in its own `finally` block -- `sort_order` is global to the
// tag library (unlike a per-video tag assignment), so leaving a test's seeded
// order in place would leak into every e2e spec that runs afterward against
// this same, persistent app.db.
export function getTagSortOrders() {
  const db = new DatabaseSync(appDbPath());
  try {
    return db.prepare("SELECT id, sort_order FROM tags").all();
  } finally {
    db.close();
  }
}

// Restores a snapshot from getTagSortOrders(). Tags created after the snapshot
// (the test's own seeded ones) are reset to NULL.
export function restoreTagSortOrders(snapshot) {
  const db = new DatabaseSync(appDbPath());
  try {
    db.exec("UPDATE tags SET sort_order = NULL");
    const stmt = db.prepare("UPDATE tags SET sort_order = ? WHERE id = ?");
    for (const { id, sort_order } of snapshot) {
      stmt.run(sort_order, id);
    }
  } finally {
    db.close();
  }
}

// Gives `tagIds` sort_order 0.. in the given order; every other tag becomes
// NULL (= after the ordered ones, by name), the same shape set_tag_order
// leaves behind.
export function seedTagOrder(tagIds) {
  const db = new DatabaseSync(appDbPath());
  try {
    db.exec("UPDATE tags SET sort_order = NULL");
    const stmt = db.prepare("UPDATE tags SET sort_order = ? WHERE id = ?");
    tagIds.forEach((id, index) => stmt.run(index, id));
  } finally {
    db.close();
  }
}
