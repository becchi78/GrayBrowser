// Generates the updater manifest (latest.json) attached to each release.
// Same shape as release-notes.mjs: dependency-free pure functions that can be
// unit-tested, with the same file doubling as the CLI the workflow calls.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { extractChangelogSection, installerFileName, versionFromTag } from "./release-notes.mjs";

const REPOSITORY = "becchi78/GrayBrowser";
const PLATFORM = "windows-x86_64";

// The preview channel is a fixed release whose tag never changes; the app
// reads its latest.json from a hardcoded URL, so the tag name is part of the
// contract with already-distributed clients.
export const PREVIEW_TAG = "dev";

export function installerUrl(tag, version) {
  return `https://github.com/${REPOSITORY}/releases/download/${tag}/${installerFileName(version)}`;
}

/**
 * Reads the contents of a `.sig` file. The updater stores it as the manifest
 * `signature` verbatim, so only surrounding whitespace is removed. An empty
 * file means signing silently produced nothing, which must stop the release
 * rather than ship a manifest every client would reject.
 */
export function normalizeSignature(raw) {
  const signature = raw.trim();
  if (signature === "") {
    throw new Error("署名ファイル(.sig)が空です");
  }
  // The file is base64 text. Anything else (e.g. a pasted private key or an
  // error message) must never be embedded into a published manifest.
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(signature)) {
    throw new Error("署名ファイル(.sig)の内容が base64 ではありません");
  }
  return signature;
}

/**
 * Highest migration version defined in the Rust migration list. The source of
 * truth is `MIGRATIONS` itself (a version missing from it is never applied),
 * so it is read from there instead of from the SQL file names, which could be
 * ahead of the list. Only the text before the test module is scanned, and zero
 * matches is an error: a silent 0 would publish "schema version 0", which the
 * app would read as "every database is too new for this release".
 */
export function latestMigrationVersion(rustSource) {
  const production = rustSource.split("#[cfg(test)]")[0];
  const versions = [...production.matchAll(/Migration\s*\{\s*version:\s*(\d+)\s*,/g)].map((m) => Number(m[1]));
  if (versions.length === 0) {
    throw new Error("migrations.rs からマイグレーションの版を読み取れませんでした");
  }
  return Math.max(...versions);
}

/** Migration file numbers in `src-tauri/migrations` (`0013_*.sql` -> 13); used to cross-check the list. */
export function migrationFileVersions(fileNames) {
  return fileNames.flatMap((name) => {
    const m = /^(\d+)_.*\.sql$/.exec(name);
    return m ? [Number(m[1])] : [];
  });
}

/**
 * Assembles the manifest. Throws when the CHANGELOG has no (or an empty) section for the version.
 *
 * `schemaVersion` is the DB schema version this release's app migrates to. It
 * lives under a `graybrowser` key of its own because the updater ignores
 * unknown fields (and keeps them in `raw_json`), so the app can warn before a
 * downgrade to a build that cannot open the current database.
 */
export function buildManifest({ tag, changelog, signature, pubDate, schemaVersion, channel = "stable" }) {
  if (channel !== "stable" && channel !== "preview") {
    throw new Error(`channel "${channel}" は stable または preview ではありません`);
  }
  const version = versionFromTag(tag);
  if (version === null) {
    throw new Error(`タグ "${tag}" は vX.Y.Z 形式ではありません`);
  }
  // Same CHANGELOG section for both channels: the preview build is the very
  // build that becomes the stable release.
  const notes = extractChangelogSection(changelog, version);
  if (notes === null) {
    throw new Error(`CHANGELOG.md に "## [${version}]" の項がありません`);
  }
  if (notes === "") {
    throw new Error(`CHANGELOG.md の "## [${version}]" の項が空です`);
  }
  if (!Number.isInteger(schemaVersion) || schemaVersion < 1) {
    throw new Error(`schemaVersion "${schemaVersion}" は 1 以上の整数ではありません`);
  }
  return {
    version,
    notes,
    pub_date: pubDate,
    graybrowser: { schemaVersion },
    platforms: {
      [PLATFORM]: {
        signature: normalizeSignature(signature),
        url: installerUrl(channel === "preview" ? PREVIEW_TAG : tag, version),
      },
    },
  };
}

const USAGE = "usage: node update-manifest.mjs vX.Y.Z <installer.sig path> [--channel stable|preview]";

/**
 * Parses `<tag> <sig path> [--channel stable|preview]`. Returns null for anything
 * else (missing positionals, a bare `--channel`, an unknown value, other order),
 * so a typo cannot silently fall back to the stable URL.
 */
export function parseArgs(argv) {
  const [tag, signaturePath, ...rest] = argv;
  if (!tag || !signaturePath) return null;
  if (rest.length === 0) return { tag, signaturePath, channel: "stable" };
  if (rest.length === 2 && rest[0] === "--channel" && (rest[1] === "stable" || rest[1] === "preview")) {
    return { tag, signaturePath, channel: rest[1] };
  }
  return null;
}

function main(argv) {
  const args = parseArgs(argv);
  if (args === null) {
    console.error(USAGE);
    return 2;
  }
  const { tag, signaturePath, channel } = args;
  const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
  try {
    const manifest = buildManifest({
      tag,
      changelog: readFileSync(join(root, "CHANGELOG.md"), "utf8"),
      signature: readFileSync(signaturePath, "utf8"),
      // toISOString() is RFC 3339 in UTC, which is what the updater parses.
      pubDate: new Date().toISOString(),
      schemaVersion: latestMigrationVersion(readFileSync(join(root, "src-tauri", "src", "db", "migrations.rs"), "utf8")),
      channel,
    });
    process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
    return 0;
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    return 1;
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exitCode = main(process.argv.slice(2));
}
