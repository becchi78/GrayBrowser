// Generates the updater manifest (latest.json) attached to each release.
// Same shape as release-notes.mjs: dependency-free pure functions that can be
// unit-tested, with the same file doubling as the CLI the workflow calls.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { extractChangelogSection, installerFileName, versionFromTag } from "./release-notes.mjs";

const REPOSITORY = "becchi78/GrayBrowser";
const PLATFORM = "windows-x86_64";

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

/** Assembles the manifest. Throws when the CHANGELOG has no (or an empty) section for the version. */
export function buildManifest({ tag, changelog, signature, pubDate }) {
  const version = versionFromTag(tag);
  if (version === null) {
    throw new Error(`タグ "${tag}" は vX.Y.Z 形式ではありません`);
  }
  const notes = extractChangelogSection(changelog, version);
  if (notes === null) {
    throw new Error(`CHANGELOG.md に "## [${version}]" の項がありません`);
  }
  if (notes === "") {
    throw new Error(`CHANGELOG.md の "## [${version}]" の項が空です`);
  }
  return {
    version,
    notes,
    pub_date: pubDate,
    platforms: {
      [PLATFORM]: {
        signature: normalizeSignature(signature),
        url: installerUrl(tag, version),
      },
    },
  };
}

function main(argv) {
  const [tag, signaturePath] = argv;
  if (!tag || !signaturePath) {
    console.error("usage: node update-manifest.mjs vX.Y.Z <installer.sig path>");
    return 2;
  }
  const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
  try {
    const manifest = buildManifest({
      tag,
      changelog: readFileSync(join(root, "CHANGELOG.md"), "utf8"),
      signature: readFileSync(signaturePath, "utf8"),
      // toISOString() is RFC 3339 in UTC, which is what the updater parses.
      pubDate: new Date().toISOString(),
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
