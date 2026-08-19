// tauri-driver connection helper. tauri-driver proxies the WebDriver
// protocol to WebView2Driver against a real, already-built GrayBrowser.exe
// -- there is no dev-server/hot-reload involved here, unlike
// `cargo tauri dev`.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { remote } from "webdriverio";
import { assertSandbox } from "./sandboxGuard.mjs";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCREENSHOTS_DIR = path.join(REPO_ROOT, "e2e", "screenshots");

// CI's e2e job reuses the release build already produced by the `build` job
// (ci.yml comment: avoids a second full Tauri build) rather than doing its
// own separate --debug build, so this defaults to the release path to match
// what CI actually exercises. Override locally with GB_APP_PATH if you built
// a debug binary instead.
const DEFAULT_APP_PATH = path.join(REPO_ROOT, "target", "release", "GrayBrowser.exe");
export const APP_PATH = process.env.GB_APP_PATH ?? DEFAULT_APP_PATH;

// GB_APP_PATHが実データ（開発機のtarget/{debug,release}/GrayBrowser/app.db）
// を指していないことを、このモジュールの読み込み時点（アプリ起動やDB書き込み
// より前）で検証する。fixtures.mjsのseedWatchFolder()はcreateSession()より
// 先にappDbPath()へ書き込むため、ガードはここに置く必要がある。
assertSandbox(process.env.GB_APP_PATH);

export function appDbPath() {
  return path.join(path.dirname(APP_PATH), "GrayBrowser", "app.db");
}

export async function createSession() {
  return remote({
    hostname: "localhost",
    port: 4444,
    connectionRetryTimeout: 30_000,
    capabilities: {
      "tauri:options": {
        application: APP_PATH,
      },
    },
  });
}

// ci.yml's "Upload E2E failure screenshots" step uploads from this
// directory on failure -- created on demand here (never committed, and
// never even created on a passing run) rather than as a placeholder in git.
export async function saveFailureScreenshot(browser, name) {
  fs.mkdirSync(SCREENSHOTS_DIR, { recursive: true });
  const file = path.join(SCREENSHOTS_DIR, `${name}-${Date.now()}.png`);
  await browser.saveScreenshot(file).catch(() => {});
}
