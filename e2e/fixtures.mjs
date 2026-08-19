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
