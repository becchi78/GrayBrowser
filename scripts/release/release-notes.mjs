// Release note generation for the tag-triggered release workflow.
// Kept free of dependencies and side effects on import so the pure functions
// can be unit-tested and the same file doubles as a CLI.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

// Prerelease/build suffixes are rejected on purpose: the workflow publishes
// stable installers only, and a loose parse would let "v1.2.3-rc1" pass the
// consistency check against a plain "1.2.3" in the manifests.
const TAG_PATTERN = /^v(\d+)\.(\d+)\.(\d+)$/;

/** "v0.8.4" -> "0.8.4". Returns null for anything that is not vX.Y.Z. */
export function versionFromTag(tag) {
  const match = TAG_PATTERN.exec(tag);
  return match ? `${match[1]}.${match[2]}.${match[3]}` : null;
}

function splitLines(text) {
  // The working tree may hold CRLF (autocrlf checkouts), and the notes must
  // not carry stray \r into GitHub's rendering.
  return text.split(/\r?\n/);
}

/**
 * Returns the body of the "## [version]" section of a Keep-a-Changelog file
 * (heading line excluded, surrounding blank lines trimmed), or null when the
 * section does not exist.
 */
export function extractChangelogSection(changelog, version) {
  const lines = splitLines(changelog);
  const start = lines.findIndex((line) => {
    const match = /^## \[([^\]]+)\]/.exec(line);
    return match !== null && match[1] === version;
  });
  if (start === -1) return null;
  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    // "## " only: "### " subsections belong to the current section.
    if (lines[i].startsWith("## ")) {
      end = i;
      break;
    }
  }
  return lines.slice(start + 1, end).join("\n").trim();
}

/** Promotes "### " subsection headings to "## " (the hand-written notes use "## 追加" etc.). */
export function promoteHeadings(body) {
  return splitLines(body)
    .map((line) => (line.startsWith("### ") ? line.slice(1) : line))
    .join("\n");
}

export function installerFileName(version) {
  return `GrayBrowser_${version}_x64-setup.exe`;
}

/** Uppercase hex, matching the hashes in the existing release notes. */
export function sha256Hex(bytes) {
  return createHash("sha256").update(bytes).digest("hex").toUpperCase();
}

/** Assembles the Draft release body. Throws when the version has no CHANGELOG section. */
export function buildReleaseNotes({ changelog, version, installerName, sha256 }) {
  const section = extractChangelogSection(changelog, version);
  if (section === null) {
    throw new Error(`CHANGELOG.md に "## [${version}]" の項がありません`);
  }
  if (section === "") {
    throw new Error(`CHANGELOG.md の "## [${version}]" の項が空です`);
  }
  return [
    promoteHeadings(section),
    "",
    "## インストール時の注意(Windows SmartScreen)",
    "",
    "インストーラーはコード署名をしていないため、実行時に Windows SmartScreen の警告が表示される場合があります。「詳細情報」→「実行」の順にクリックして進めてください。ダウンロードは、このリポジトリの Releases ページのファイルから行ってください。",
    "",
    "ダウンロードしたファイルは、PowerShell の `Get-FileHash -Algorithm SHA256 <ファイル名>` で算出したハッシュが、下記の値と一致することを確認できます。",
    "",
    "## SHA256",
    "",
    `\`${installerName}\`: \`${sha256}\``,
    "",
  ].join("\n");
}

function main(argv) {
  const [tag, installerPath] = argv;
  if (!tag) {
    console.error("usage: node release-notes.mjs vX.Y.Z [installer-path]");
    return 2;
  }
  const version = versionFromTag(tag);
  if (version === null) {
    console.error(`タグ "${tag}" は vX.Y.Z 形式ではありません`);
    return 1;
  }
  const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
  try {
    const changelog = readFileSync(join(root, "CHANGELOG.md"), "utf8");
    // Without an installer (local preview) the hash is a visible placeholder
    // rather than a made-up value.
    const installerName = installerPath ? basename(installerPath) : installerFileName(version);
    const sha256 = installerPath ? sha256Hex(readFileSync(installerPath)) : "<SHA256>";
    process.stdout.write(buildReleaseNotes({ changelog, version, installerName, sha256 }));
    return 0;
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    return 1;
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exitCode = main(process.argv.slice(2));
}
