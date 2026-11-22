import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  buildManifest,
  installerUrl,
  latestMigrationVersion,
  migrationFileVersions,
  normalizeSignature,
  parseArgs,
  PREVIEW_TAG,
} from "./update-manifest.mjs";

const CHANGELOG = [
  "# Changelog",
  "",
  "## [0.2.0] - 2026-01-02",
  "",
  "### 追加",
  "",
  "- 新機能",
  "",
  "## [0.1.0] - 2026-01-01",
  "",
  "### 追加",
  "",
  "- 最初の版",
  "",
].join("\n");

const SIG = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZw==";
const SCHEMA = 13;

test("installerUrl points at the tagged release asset", () => {
  assert.equal(
    installerUrl("v0.2.0", "0.2.0"),
    "https://github.com/becchi78/GrayBrowser/releases/download/v0.2.0/GrayBrowser_0.2.0_x64-setup.exe",
  );
});

test("buildManifest has the fields the updater reads", () => {
  const manifest = buildManifest({
    tag: "v0.2.0",
    changelog: CHANGELOG,
    signature: `${SIG}\n`,
    pubDate: "2026-01-02T03:04:05.000Z",
    schemaVersion: SCHEMA,
  });
  assert.deepEqual(manifest, {
    version: "0.2.0",
    notes: "### 追加\n\n- 新機能",
    pub_date: "2026-01-02T03:04:05.000Z",
    graybrowser: { schemaVersion: SCHEMA },
    platforms: {
      "windows-x86_64": {
        signature: SIG,
        url: "https://github.com/becchi78/GrayBrowser/releases/download/v0.2.0/GrayBrowser_0.2.0_x64-setup.exe",
      },
    },
  });
});

test("buildManifest takes only the tagged version's notes", () => {
  const manifest = buildManifest({ tag: "v0.1.0", changelog: CHANGELOG, signature: SIG, pubDate: "x", schemaVersion: SCHEMA });
  assert.equal(manifest.notes, "### 追加\n\n- 最初の版");
});

test("buildManifest fails for a missing or empty CHANGELOG section", () => {
  const args = { signature: SIG, pubDate: "x", schemaVersion: SCHEMA };
  assert.throws(() => buildManifest({ ...args, tag: "v9.9.9", changelog: CHANGELOG }), /9\.9\.9/);
  assert.throws(
    () => buildManifest({ ...args, tag: "v1.0.0", changelog: "## [1.0.0] - x\n\n## [0.9.0] - y\n- a\n" }),
    /空/,
  );
});

test("buildManifest rejects tags that are not vX.Y.Z", () => {
  for (const tag of ["0.2.0", "v0.2", "v0.2.0-rc1", ""]) {
    assert.throws(() => buildManifest({ tag, changelog: CHANGELOG, signature: SIG, pubDate: "x", schemaVersion: SCHEMA }), /vX\.Y\.Z/, tag);
  }
});

test("buildManifest fails without a usable signature", () => {
  const args = { tag: "v0.2.0", changelog: CHANGELOG, pubDate: "x", schemaVersion: SCHEMA };
  assert.throws(() => buildManifest({ ...args, signature: "" }), /空/);
  assert.throws(() => buildManifest({ ...args, signature: " \r\n" }), /空/);
});

test("normalizeSignature trims whitespace, including CRLF", () => {
  assert.equal(normalizeSignature(`  ${SIG}\r\n`), SIG);
});

test("normalizeSignature refuses text that is not base64", () => {
  for (const bad of ["untrusted comment: x\nabc", "error: failed to sign", "abc def", "ab-cd"]) {
    assert.throws(() => normalizeSignature(bad), /base64/, bad);
  }
});

test("buildManifest rejects dev tags", () => {
  for (const tag of ["v0.3.0-dev.2", "v0.3.0-dev", "v0.3.0-dev.01"]) {
    assert.throws(() => buildManifest({ tag, changelog: CHANGELOG, signature: SIG, pubDate: "x", schemaVersion: SCHEMA }), /vX\.Y\.Z/, tag);
  }
});

test("buildManifest for the preview channel points into the fixed dev release", () => {
  const args = { tag: "v0.2.0", changelog: CHANGELOG, signature: SIG, pubDate: "2026-01-02T03:04:05.000Z", schemaVersion: SCHEMA };
  const stable = buildManifest(args);
  const preview = buildManifest({ ...args, channel: "preview" });
  assert.equal(PREVIEW_TAG, "dev");
  assert.equal(
    preview.platforms["windows-x86_64"].url,
    "https://github.com/becchi78/GrayBrowser/releases/download/dev/GrayBrowser_0.2.0_x64-setup.exe",
  );
  // Everything except the URL is identical, so the signature stays valid.
  assert.equal(preview.version, stable.version);
  assert.equal(preview.notes, stable.notes);
  assert.equal(preview.platforms["windows-x86_64"].signature, stable.platforms["windows-x86_64"].signature);
});

test("buildManifest rejects an unknown channel", () => {
  assert.throws(
    () => buildManifest({ tag: "v0.2.0", changelog: CHANGELOG, signature: SIG, pubDate: "x", schemaVersion: SCHEMA, channel: "beta" }),
    /channel/,
  );
});

test("parseArgs defaults to stable and accepts --channel after the positionals", () => {
  assert.deepEqual(parseArgs(["v1.0.0", "a.sig"]), { tag: "v1.0.0", signaturePath: "a.sig", channel: "stable" });
  assert.deepEqual(parseArgs(["v1.0.0", "a.sig", "--channel", "preview"]), {
    tag: "v1.0.0",
    signaturePath: "a.sig",
    channel: "preview",
  });
  assert.equal(parseArgs(["v1.0.0", "a.sig", "--channel", "stable"]).channel, "stable");
});

test("parseArgs rejects a bare --channel, unknown values, wrong order and extra arguments", () => {
  const bad = [
    [],
    ["v1.0.0"],
    ["v1.0.0", "a.sig", "--channel"],
    ["v1.0.0", "a.sig", "--channel", "beta"],
    ["v1.0.0", "a.sig", "--channel", ""],
    ["--channel", "preview", "v1.0.0", "a.sig"],
    ["v1.0.0", "--channel", "preview", "a.sig"],
    ["v1.0.0", "a.sig", "preview"],
    ["v1.0.0", "a.sig", "--channel", "preview", "x"],
  ];
  for (const argv of bad) assert.equal(parseArgs(argv), null, JSON.stringify(argv));
});

test("buildManifest carries the schema version for both channels", () => {
  const args = { tag: "v0.2.0", changelog: CHANGELOG, signature: SIG, pubDate: "x", schemaVersion: 7 };
  assert.deepEqual(buildManifest(args).graybrowser, { schemaVersion: 7 });
  assert.deepEqual(buildManifest({ ...args, channel: "preview" }).graybrowser, { schemaVersion: 7 });
});

test("buildManifest refuses a missing or invalid schema version", () => {
  const args = { tag: "v0.2.0", changelog: CHANGELOG, signature: SIG, pubDate: "x" };
  for (const schemaVersion of [undefined, null, 0, -1, 1.5, "13", Number.NaN]) {
    assert.throws(() => buildManifest({ ...args, schemaVersion }), /schemaVersion/, String(schemaVersion));
  }
});

test("latestMigrationVersion reads the highest production migration and ignores the test module", () => {
  const source = [
    "pub const MIGRATIONS: &[Migration] = &[",
    "    gb_core::migrations::Migration {",
    "        version: 2,",
    "        sql: \"b\",",
    "    },",
    "    gb_core::migrations::Migration {",
    "        version: 11,",
    "    },",
    "];",
    "#[cfg(test)]",
    "mod tests { const X: Migration = Migration { version: 99, sql: \"\" }; }",
  ].join("\n");
  assert.equal(latestMigrationVersion(source), 11);
});

test("latestMigrationVersion fails loudly when nothing can be read", () => {
  assert.throws(() => latestMigrationVersion("pub const MIGRATIONS: &[Migration] = &[];"), /読み取れ/);
});

test("the schema version read from migrations.rs matches the migration files in the repository", () => {
  const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const fromRust = latestMigrationVersion(readFileSync(join(root, "src-tauri", "src", "db", "migrations.rs"), "utf8"));
  const fromFiles = Math.max(...migrationFileVersions(readdirSync(join(root, "src-tauri", "migrations"))));
  assert.ok(fromRust >= 1);
  assert.equal(fromRust, fromFiles);
});
