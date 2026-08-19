// E2E / 実機確認のサンドボックス化ガード。
//
// 背景: GB_APP_PATH未設定のままE2E/実機確認を
// 実行し、session.mjsのDEFAULT_APP_PATHフォールバック（target/release/
// graybrowser.exe）経由で開発機の実データ（target/{debug,release}/
// GrayBrowser/app.db）に書き込みが発生する事故があった。このモジュールは
// GB_APP_PATHがテスト専用サンドボックスを指していることを、アプリ起動前・
// WebDriverセッション起動前に検証し、満たさなければ即座に失敗させる。
//
// 判定ロジック（evaluateSandboxGuard）は純関数として切り出し、実際のfs
// アクセス（assertSandbox）と分離してユニットテスト可能にしている。

import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

export const SENTINEL_FILENAME = ".gb-test-sandbox";

/**
 * サンドボックスガードの判定ロジック（純関数）。
 * @param {{ appPathEnv: string | undefined, dirExists: boolean, sentinelExists: boolean }} params
 * @returns {{ ok: true } | { ok: false, reason: string }}
 */
export function evaluateSandboxGuard({ appPathEnv, dirExists, sentinelExists }) {
  if (!appPathEnv) {
    return {
      ok: false,
      reason:
        "GB_APP_PATH が未設定です。E2E/実機確認は開発機の実データ" +
        "（target/{debug,release}/GrayBrowser/app.db）ではなく、隔離された" +
        "サンドボックスビルドに対して実行しなければなりません。" +
        "GB_APP_PATH にサンドボックスビルドのexeパスを設定し、その親" +
        "ディレクトリに .gb-test-sandbox ファイルを置いてください。" +
        "詳細は e2e/README.md を参照してください。",
    };
  }

  if (!dirExists) {
    return {
      ok: false,
      reason:
        `GB_APP_PATH（${appPathEnv}）の親ディレクトリが存在しません。` +
        "サンドボックスビルドを先に作成し、GB_APP_PATH にその出力先exeの" +
        "パスを設定してください。詳細は e2e/README.md を参照してください。",
    };
  }

  if (!sentinelExists) {
    return {
      ok: false,
      reason:
        `GB_APP_PATH（${appPathEnv}）の親ディレクトリに ${SENTINEL_FILENAME} ` +
        "が見つからないため、サンドボックスと確認できません。実データの" +
        "ディレクトリを誤って指定していないか確認するか、サンドボックス" +
        `ディレクトリに ${SENTINEL_FILENAME}（空ファイルで可）を作成して` +
        "ください。詳細は e2e/README.md を参照してください。",
    };
  }

  return { ok: true };
}

/**
 * 実際のfsを使ってGB_APP_PATHがサンドボックスを指していることを検証し、
 * 満たさなければ例外を投げる。
 * @param {string | undefined} appPathEnv
 */
export function assertSandbox(appPathEnv = process.env.GB_APP_PATH) {
  let dirExists = false;
  let sentinelExists = false;

  if (appPathEnv) {
    const dir = path.dirname(appPathEnv);
    dirExists = fs.existsSync(dir) && fs.statSync(dir).isDirectory();
    if (dirExists) {
      sentinelExists = fs.existsSync(path.join(dir, SENTINEL_FILENAME));
    }
  }

  const result = evaluateSandboxGuard({ appPathEnv, dirExists, sentinelExists });
  if (!result.ok) {
    throw new Error(`[E2E sandbox guard] ${result.reason}`);
  }
}

/**
 * dirPath直下にsentinelファイル（SENTINEL_FILENAME）が無ければ作成する。
 * 既に存在する場合は何もしない（内容を上書きしない）。
 *
 * sentinelファイル名のリテラルをこのモジュール（JS側）に一本化するための
 * エントリポイント。run-sandboxed.ps1（PowerShell側）はファイル名を一切
 * 知らず、このモジュールをnode経由で呼び出すことでsentinelを作成する。
 *
 * @param {string} dirPath sentinelを作成するディレクトリ
 * @returns {string} 作成した（または既存の）sentinelファイルのフルパス
 */
export function ensureSentinel(dirPath) {
  if (!dirPath || typeof dirPath !== "string") {
    throw new Error("ensureSentinel: dirPath must be a non-empty string");
  }
  if (!fs.existsSync(dirPath) || !fs.statSync(dirPath).isDirectory()) {
    throw new Error(`ensureSentinel: directory does not exist: ${dirPath}`);
  }

  const sentinelPath = path.join(dirPath, SENTINEL_FILENAME);
  if (!fs.existsSync(sentinelPath)) {
    fs.writeFileSync(sentinelPath, "");
  }
  return sentinelPath;
}

// CLIエントリポイント（`node e2e/sandboxGuard.mjs <dir>` として直接実行された
// 場合のみ発火する）。run-sandboxed.ps1 から呼び出される想定。
// 標準出力にはsentinelのフルパス1行のみを書く（PowerShell側がそれを厳密に
// 検証するため、余計な出力を混ぜない）。
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const dirArg = process.argv[2];
  if (!dirArg) {
    process.stderr.write(
      "Usage: node e2e/sandboxGuard.mjs <dir>\n" +
        "  <dir> にsentinelファイル（.gb-test-sandbox）を作成し、そのフルパスを標準出力に書く。\n",
    );
    process.exit(1);
  }

  try {
    const sentinelPath = ensureSentinel(dirArg);
    process.stdout.write(`${sentinelPath}\n`);
    process.exit(0);
  } catch (err) {
    process.stderr.write(`[sandboxGuard] ensureSentinel failed: ${err.message}\n`);
    process.exit(1);
  }
}
