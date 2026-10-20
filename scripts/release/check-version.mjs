// Pre-build gate for the release workflow: the tag, the three manifest
// versions and the CHANGELOG must all agree before any installer is built.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { extractChangelogSection, versionFromTag } from "./release-notes.mjs";

/** `version` of the `[package]` table; other tables' `version` keys are ignored. */
export function cargoPackageVersion(cargoToml) {
  let inPackage = false;
  for (const line of cargoToml.split(/\r?\n/)) {
    const table = /^\s*\[([^\]]+)\]\s*(#.*)?$/.exec(line);
    if (table) {
      inPackage = table[1].trim() === "package";
      continue;
    }
    if (inPackage) {
      const match = /^\s*version\s*=\s*"([^"]*)"/.exec(line);
      if (match) return match[1];
    }
  }
  return null;
}

/**
 * Returns the list of problems (empty when everything is consistent).
 * `versions` maps a display label to the version found there (null = missing).
 */
export function checkVersions({ tag, versions, changelog }) {
  const expected = versionFromTag(tag);
  if (expected === null) {
    return [`タグ "${tag}" は vX.Y.Z 形式ではありません(プレリリース形式は対象外)`];
  }
  const problems = [];
  for (const [label, found] of Object.entries(versions)) {
    if (found !== expected) {
      problems.push(`${label} のバージョン(${found ?? "取得できず"})がタグ(${expected})と一致しません`);
    }
  }
  if (extractChangelogSection(changelog, expected) === null) {
    problems.push(`CHANGELOG.md に "## [${expected}]" の項がありません`);
  }
  return problems;
}

function main(argv) {
  const [tag] = argv;
  if (!tag) {
    console.error("usage: node check-version.mjs vX.Y.Z");
    return 2;
  }
  const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const read = (path) => readFileSync(join(root, path), "utf8");
  let problems;
  try {
    problems = checkVersions({
      tag,
      versions: {
        "package.json": JSON.parse(read("package.json")).version ?? null,
        "src-tauri/tauri.conf.json": JSON.parse(read("src-tauri/tauri.conf.json")).version ?? null,
        "src-tauri/Cargo.toml": cargoPackageVersion(read("src-tauri/Cargo.toml")),
      },
      changelog: read("CHANGELOG.md"),
    });
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    return 1;
  }
  if (problems.length > 0) {
    for (const problem of problems) console.error(`error: ${problem}`);
    return 1;
  }
  console.log(`ok: ${tag} はすべてのバージョンと CHANGELOG に一致しています`);
  return 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  process.exitCode = main(process.argv.slice(2));
}
