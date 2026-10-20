import assert from "node:assert/strict";
import test from "node:test";
import { buildManifest, installerUrl, normalizeSignature } from "./update-manifest.mjs";

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
  });
  assert.deepEqual(manifest, {
    version: "0.2.0",
    notes: "### 追加\n\n- 新機能",
    pub_date: "2026-01-02T03:04:05.000Z",
    platforms: {
      "windows-x86_64": {
        signature: SIG,
        url: "https://github.com/becchi78/GrayBrowser/releases/download/v0.2.0/GrayBrowser_0.2.0_x64-setup.exe",
      },
    },
  });
});

test("buildManifest takes only the tagged version's notes", () => {
  const manifest = buildManifest({ tag: "v0.1.0", changelog: CHANGELOG, signature: SIG, pubDate: "x" });
  assert.equal(manifest.notes, "### 追加\n\n- 最初の版");
});

test("buildManifest fails for a missing or empty CHANGELOG section", () => {
  const args = { signature: SIG, pubDate: "x" };
  assert.throws(() => buildManifest({ ...args, tag: "v9.9.9", changelog: CHANGELOG }), /9\.9\.9/);
  assert.throws(
    () => buildManifest({ ...args, tag: "v1.0.0", changelog: "## [1.0.0] - x\n\n## [0.9.0] - y\n- a\n" }),
    /空/,
  );
});

test("buildManifest rejects tags that are not vX.Y.Z", () => {
  for (const tag of ["0.2.0", "v0.2", "v0.2.0-rc1", ""]) {
    assert.throws(() => buildManifest({ tag, changelog: CHANGELOG, signature: SIG, pubDate: "x" }), /vX\.Y\.Z/, tag);
  }
});

test("buildManifest fails without a usable signature", () => {
  const args = { tag: "v0.2.0", changelog: CHANGELOG, pubDate: "x" };
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
