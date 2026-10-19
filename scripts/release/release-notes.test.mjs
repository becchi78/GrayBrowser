import assert from "node:assert/strict";
import test from "node:test";
import {
  buildReleaseNotes,
  extractChangelogSection,
  installerFileName,
  promoteHeadings,
  sha256Hex,
  versionFromTag,
} from "./release-notes.mjs";

const CHANGELOG = [
  "# Changelog",
  "",
  "前文",
  "",
  "## [0.2.0] - 2026-01-02",
  "",
  "### 追加",
  "",
  "- 新機能",
  "",
  "### 修正",
  "",
  "- 不具合",
  "",
  "## [0.1.0] - 2026-01-01",
  "",
  "### 追加",
  "",
  "- 最初の版",
  "",
].join("\n");

test("versionFromTag accepts only vX.Y.Z", () => {
  assert.equal(versionFromTag("v0.8.4"), "0.8.4");
  assert.equal(versionFromTag("v10.20.30"), "10.20.30");
  for (const bad of ["0.8.4", "v0.8", "v0.8.4-rc1", "v0.8.4+build", "v0.8.4.1", "vx.y.z", "", "v0.8.4 "]) {
    assert.equal(versionFromTag(bad), null, bad);
  }
});

test("extractChangelogSection picks the right section among several", () => {
  const section = extractChangelogSection(CHANGELOG, "0.2.0");
  assert.equal(section, "### 追加\n\n- 新機能\n\n### 修正\n\n- 不具合");
  assert.ok(!section.includes("最初の版"));
});

test("extractChangelogSection handles the last section", () => {
  assert.equal(extractChangelogSection(CHANGELOG, "0.1.0"), "### 追加\n\n- 最初の版");
});

test("extractChangelogSection returns null when missing, without prefix matching", () => {
  assert.equal(extractChangelogSection(CHANGELOG, "0.3.0"), null);
  assert.equal(extractChangelogSection(CHANGELOG, "0.2"), null);
  assert.equal(extractChangelogSection(CHANGELOG, "0.2.00"), null);
  // "." must not act as a regex wildcard.
  assert.equal(extractChangelogSection(CHANGELOG, "0x2x0"), null);
});

test("extractChangelogSection tolerates CRLF without leaking \r", () => {
  const section = extractChangelogSection(CHANGELOG.replace(/\n/g, "\r\n"), "0.2.0");
  assert.equal(section, "### 追加\n\n- 新機能\n\n### 修正\n\n- 不具合");
});

test("extractChangelogSection ignores a version mentioned only in text", () => {
  assert.equal(extractChangelogSection("# Changelog\n\n0.9.9 を参照\n", "0.9.9"), null);
});

test("promoteHeadings turns ### into ## and leaves other lines alone", () => {
  assert.equal(promoteHeadings("### 追加\n- a\n#### 小\n## 既に\n本文 ### 途中"), "## 追加\n- a\n#### 小\n## 既に\n本文 ### 途中");
});

test("buildReleaseNotes assembles changes, SmartScreen and SHA256 sections", () => {
  const notes = buildReleaseNotes({
    changelog: CHANGELOG,
    version: "0.2.0",
    installerName: installerFileName("0.2.0"),
    sha256: "ABC123",
  });
  assert.ok(notes.startsWith("## 追加\n\n- 新機能\n\n## 修正\n\n- 不具合\n\n## インストール時の注意(Windows SmartScreen)\n"));
  assert.ok(!notes.includes("###"));
  assert.ok(notes.includes("Windows SmartScreen の警告"));
  assert.ok(notes.endsWith("\n## SHA256\n\n`GrayBrowser_0.2.0_x64-setup.exe`: `ABC123`\n"));
});

test("buildReleaseNotes throws for a missing or empty section", () => {
  const args = { installerName: "x.exe", sha256: "00" };
  assert.throws(() => buildReleaseNotes({ ...args, changelog: CHANGELOG, version: "9.9.9" }), /9\.9\.9/);
  assert.throws(
    () => buildReleaseNotes({ ...args, changelog: "## [1.0.0] - x\n\n## [0.9.0] - y\n- a\n", version: "1.0.0" }),
    /空/,
  );
});

test("sha256Hex is uppercase hex of the bytes", () => {
  assert.equal(sha256Hex(Buffer.from("abc")), "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD");
});
