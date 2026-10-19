import assert from "node:assert/strict";
import test from "node:test";
import { cargoPackageVersion, checkVersions } from "./check-version.mjs";

const CHANGELOG = "# Changelog\n\n## [1.2.3] - 2026-01-01\n\n### 追加\n\n- a\n";
const OK = { "package.json": "1.2.3", "src-tauri/tauri.conf.json": "1.2.3", "src-tauri/Cargo.toml": "1.2.3" };

test("cargoPackageVersion reads [package] only", () => {
  const toml = [
    "[package]",
    'name = "x"',
    'version = "1.2.3"',
    "",
    "[dependencies]",
    'serde = { version = "9.9.9" }',
    "",
    "[dev-dependencies.foo]",
    'version = "8.8.8"',
  ].join("\n");
  assert.equal(cargoPackageVersion(toml), "1.2.3");
});

test("cargoPackageVersion ignores version keys before [package] and in later tables", () => {
  assert.equal(cargoPackageVersion('[lib]\nversion = "7.7.7"\n[package]\nname = "x"\n'), null);
  assert.equal(cargoPackageVersion('[package]\nname = "x"\n[profile.release]\nversion = "7.7.7"\n'), null);
});

test("cargoPackageVersion handles CRLF and a trailing comment on the table header", () => {
  assert.equal(cargoPackageVersion('[package] # app\r\nname = "x"\r\nversion = "0.8.4"\r\n'), "0.8.4");
});

test("checkVersions passes when all agree", () => {
  assert.deepEqual(checkVersions({ tag: "v1.2.3", versions: OK, changelog: CHANGELOG }), []);
});

test("checkVersions reports each mismatching file", () => {
  const problems = checkVersions({
    tag: "v1.2.3",
    versions: { ...OK, "package.json": "1.2.2", "src-tauri/Cargo.toml": null },
    changelog: CHANGELOG,
  });
  assert.equal(problems.length, 2);
  assert.match(problems[0], /package\.json/);
  assert.match(problems[1], /Cargo\.toml/);
});

test("checkVersions reports a missing CHANGELOG section", () => {
  const problems = checkVersions({ tag: "v1.2.3", versions: OK, changelog: "# Changelog\n\n## [1.2.2] - x\n" });
  assert.equal(problems.length, 1);
  assert.match(problems[0], /CHANGELOG/);
});

test("checkVersions rejects prerelease and malformed tags", () => {
  for (const tag of ["v1.2.3-rc1", "1.2.3", "v1.2", "v1.2.3+b"]) {
    const problems = checkVersions({ tag, versions: OK, changelog: CHANGELOG });
    assert.equal(problems.length, 1, tag);
    assert.match(problems[0], /vX\.Y\.Z/);
  }
});

test("checkVersions works with CRLF CHANGELOG", () => {
  assert.deepEqual(checkVersions({ tag: "v1.2.3", versions: OK, changelog: CHANGELOG.replace(/\n/g, "\r\n") }), []);
});
