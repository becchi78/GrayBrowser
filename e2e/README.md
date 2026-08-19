# E2E / 実機確認の実行方法

## 背景

過去に `GB_APP_PATH` を設定しないまま E2E / 実機確認を実行し、
`e2e/session.mjs` の既定値（`target/release/graybrowser.exe`）へのフォール
バック経由で、開発機の実データ（`target/{debug,release}/GrayBrowser/app.db`）
に書き込みが発生する事故があった。以降、
E2E / 実機確認は必ず隔離されたサンドボックスビルドに対して実行する。

## ガードの仕組み

`e2e/session.mjs` は、モジュールが読み込まれた時点（アプリ起動・WebDriver
セッション起動・DBへの書き込みが行われるより前）で `e2e/sandboxGuard.mjs`
の `assertSandbox()` を呼び出し、以下をすべて満たさない限り即座に例外を投げて
処理を止める。

1. `GB_APP_PATH` 環境変数が設定されていること
2. `GB_APP_PATH` の親ディレクトリが実在すること
3. その親ディレクトリに sentinel ファイル `.gb-test-sandbox`（空ファイルで可）
   が存在すること

`e2e/fixtures.mjs`（`seedWatchFolder()` で `session.mjs` の `appDbPath()` に
直接INSERTする）を含め、`e2e/specs/*.e2e.mjs` はすべて `session.mjs` を
静的importするため、このガードはテスト本体・DB書き込み・WebDriverセッション
生成のいずれよりも先に働く。

## ローカルでの実行方法

隔離ビルド・sentinel作成・`GB_APP_PATH` 設定をまとめて行うヘルパーとして
`e2e/run-sandboxed.ps1` を用意している。既存の `target/` ディレクトリ（実データ
を含みうる）には一切触れず、`CARGO_TARGET_DIR` を一時ディレクトリ
（既定 `%TEMP%\gb-e2e-sandbox`）に切り替えてビルドする。

**`pwsh`（PowerShell 7 以上）での実行が必須。** Windows PowerShell 5.1
（`powershell.exe`）では実行できない（下記「PowerShell バージョンについて」を
参照）。

### E2Eテストを自動実行する

```powershell
./e2e/run-sandboxed.ps1
# 明示指定する場合:
./e2e/run-sandboxed.ps1 -Mode e2e -Profile release
```

release ビルド（`npm run tauri build -- --no-bundle`、インストーラ生成は
スキップ）を作成し、tauri-driver を起動して `npm run test:e2e` を実行する。

### 手動で実機確認する

```powershell
./e2e/run-sandboxed.ps1 -Mode manual -Profile debug
```

debug ビルド（`npm run build` → `cargo build -p graybrowser`）を作成し、
アプリを起動する。ウィンドウを閉じるまでスクリプトは待機する。tauri-driver
は起動しない。

### パラメータ

| パラメータ | 既定値 | 説明 |
| --- | --- | --- |
| `-Mode` | `e2e` | `e2e`（自動テスト実行）または `manual`（手動起動） |
| `-Profile` | `release` | `release` または `debug` |
| `-SandboxRoot` | `%TEMP%\gb-e2e-sandbox` | ビルド成果物の隔離先ルート |

## PowerShell バージョンについて

`e2e/run-sandboxed.ps1` は UTF-8（BOM付き）で保存されているが、日本語コメントを
含む本スクリプトを Windows PowerShell 5.1（`powershell.exe`）でそのまま実行する
と、5.1 のパーサがトークン化に失敗して実行できない。スクリプト冒頭
には PowerShell 5.1 実行時に明示的なエラーメッセージで停止するバージョンチェッ
クを入れているが、**`pwsh`（PowerShell 7 以上）で実行することが前提**であり、
`powershell.exe` から呼び出さないこと。バージョンチェックに引っかかった場合でも
`GB_APP_PATH` を手動で設定して回避しないこと（このスクリプトが提供する安全機構
そのものを無効化してしまう）。

```powershell
pwsh -File e2e/run-sandboxed.ps1
```

## 実データに触れない前提の作業を始める前に

本ガード自体の検証作業など、「実データに触れないはず」という前提で作業を
開始する場合は、念のため作業開始前に対象パス（`target/{debug,release}/
GrayBrowser/app.db`、`WhiteBrowser_test/` 配下等）の mtime を記録しておく
こと。作業完了後に再取得して照合し、無変化であることを確認する。DBはWAL
モードのため、書き込みはまず `app.db-wal` に入り `app.db` 本体の mtime が
動かないことがある。記録対象には `app.db` 本体に加え `app.db-wal` /
`app.db-shm` も含めること。

以下は対象パスの `LastWriteTime` を一覧表示する PowerShell コマンド例
（作業前後で実行して差分がないか比較する）。

```powershell
Get-Item `
  "target/debug/GrayBrowser/app.db", "target/debug/GrayBrowser/app.db-wal", "target/debug/GrayBrowser/app.db-shm", `
  "target/release/GrayBrowser/app.db", "target/release/GrayBrowser/app.db-wal", "target/release/GrayBrowser/app.db-shm", `
  "WhiteBrowser_test/*.wb" `
  -ErrorAction SilentlyContinue | Select-Object FullName, LastWriteTime
```

**mtime照合はセッション内で完結する前後比較として行うこと。**作業開始直前に取得し、作業終了直後に比較する。セッションをまたいだ比較（前回セッション終了時点の値との比較等）を事故の根拠にしないこと。実データ保有者は自分のアプリを日常的に使うため、間に正常な使用が挟まれば必ず「変化あり」になる。

**変化を見つけた場合、調査を始める前に人間に「その間にアプリを使ったか」を確認すること。**実際にこの確認を飛ばして事故と誤判定した例がある（`app.db-wal`の増加を検知し、エージェントの操作ミスによる事故と推定したが、実際は実データ保有者本人がアプリを操作した結果だった）。

## 重要な申し送り: CI側の対応は未着手（別タスク）

`.github/workflows/ci.yml` の `e2e` ジョブ（`workflow_dispatch` 限定）は
現状 `GB_APP_PATH` を設定していない。このガード導入後は、CI側を変更しない
限り **`e2e` ジョブはガードによって即座に失敗するようになる**（従来の
`build` ジョブ成果物のダウンロード先をそのまま `GB_APP_PATH` にする、または
CI用にも sentinel を作成する等の対応が必要）。この対応は本タスクのスコープ
外であり、別タスクとして扱う。
