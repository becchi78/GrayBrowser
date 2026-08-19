// e2e/sandboxGuard.mjs の判定ロジック（evaluateSandboxGuard）のユニット
// テスト。実fsアクセスを伴わない純関数のみを対象とする
// （src/lib/boundedCache.test.ts と同様、Node組み込みのtest runnerを使う）。
//
// 加えて、実fsアクセスを行うassertSandbox()（実際に事故を防ぐ処理本体）に
// ついても、os.tmpdir()配下の一時ディレクトリを使った統合テストを行う。
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { SENTINEL_FILENAME, assertSandbox, ensureSentinel, evaluateSandboxGuard } from "./sandboxGuard.mjs";

test("GB_APP_PATH未設定なら失敗する", () => {
  const result = evaluateSandboxGuard({
    appPathEnv: undefined,
    dirExists: false,
    sentinelExists: false,
  });

  assert.equal(result.ok, false);
  assert.ok(result.reason.includes("GB_APP_PATH"));
});

test("GB_APP_PATHの親ディレクトリが存在しないなら失敗する", () => {
  const result = evaluateSandboxGuard({
    appPathEnv: "C:\\nonexistent\\graybrowser.exe",
    dirExists: false,
    sentinelExists: false,
  });

  assert.equal(result.ok, false);
  assert.ok(result.reason.includes("親ディレクトリ"));
});

test("sentinelファイルの無いディレクトリなら失敗する", () => {
  const result = evaluateSandboxGuard({
    appPathEnv: "C:\\some\\dir\\graybrowser.exe",
    dirExists: true,
    sentinelExists: false,
  });

  assert.equal(result.ok, false);
  assert.ok(result.reason.includes("サンドボックスと確認できません"));
});

test("正当なサンドボックス（ディレクトリとsentinelが揃っている）なら成功する", () => {
  const result = evaluateSandboxGuard({
    appPathEnv: "C:\\sandbox\\release\\graybrowser.exe",
    dirExists: true,
    sentinelExists: true,
  });

  assert.equal(result.ok, true);
});

test("assertSandbox: sentinelファイルが無いディレクトリを指すとthrowする", () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "gb-sandbox-guard-test-"));
  try {
    const appPathEnv = path.join(tmpDir, "graybrowser.exe");
    assert.throws(() => assertSandbox(appPathEnv), /sandbox guard/);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("assertSandbox: sentinelファイルがあるディレクトリを指すとthrowしない", () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "gb-sandbox-guard-test-"));
  try {
    fs.writeFileSync(path.join(tmpDir, SENTINEL_FILENAME), "");
    const appPathEnv = path.join(tmpDir, "graybrowser.exe");
    assert.doesNotThrow(() => assertSandbox(appPathEnv));
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("ensureSentinel: sentinelが無いディレクトリに作成し、フルパスを返す", () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "gb-sandbox-guard-test-"));
  try {
    const expectedPath = path.join(tmpDir, SENTINEL_FILENAME);
    assert.equal(fs.existsSync(expectedPath), false);

    const result = ensureSentinel(tmpDir);

    assert.equal(result, expectedPath);
    assert.equal(fs.existsSync(expectedPath), true);
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("ensureSentinel: 既存のsentinelがある場合は上書きせずそのまま返す", () => {
  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "gb-sandbox-guard-test-"));
  try {
    const sentinelPath = path.join(tmpDir, SENTINEL_FILENAME);
    fs.writeFileSync(sentinelPath, "existing-content");

    const result = ensureSentinel(tmpDir);

    assert.equal(result, sentinelPath);
    assert.equal(fs.readFileSync(sentinelPath, "utf8"), "existing-content");
  } finally {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});

test("ensureSentinel: 存在しないディレクトリを指すとthrowする", () => {
  const missingDir = path.join(os.tmpdir(), "gb-sandbox-guard-test-missing-dir-xyz");
  assert.throws(() => ensureSentinel(missingDir), /directory does not exist/);
});
