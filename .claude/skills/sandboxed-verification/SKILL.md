---
name: sandboxed-verification
description: GrayBrowserのE2Eテスト・実機確認を安全に実行する手順。実データ（app.db等）を書き換える事故を防ぐためのサンドボックス化手順とmtime照合プロトコルを扱う。E2Eテストを実行する、cargo tauri dev以外の方法でアプリを起動して手動確認する、実機確認を行う際に参照する。
---

# GrayBrowser サンドボックス化検証ガイド

## 背景

過去に `GB_APP_PATH` を設定しないままE2E/実機確認を実行し、開発機の実データ（`target/{debug,release}/GrayBrowser/app.db`）に書き込みが発生する事故があった（PR #19）。以降、E2E・実機確認は必ず隔離されたサンドボックスビルドに対して実行する。

## 実行手順

実行は必ず `e2e/run-sandboxed.ps1` 経由で行う。`GB_APP_PATH` を手動で設定して生exeを起動しない（このスクリプトが提供する安全機構そのものを無効化することになる）。

```powershell
# E2Eテストを自動実行
pwsh -File e2e/run-sandboxed.ps1 -Mode e2e -Profile release

# 手動で実機確認（ウィンドウが閉じるまで待機）
pwsh -File e2e/run-sandboxed.ps1 -Mode manual -Profile debug
```

**`pwsh`（PowerShell 7以上）での実行が必須。** Windows PowerShell 5.1（`powershell.exe`）ではパーサがトークン化に失敗し実行できない（Issue #33）。バージョンチェックに引っかかった場合でも、`GB_APP_PATH` を手動設定して回避しない。

詳細な仕組み（`sandboxGuard.mjs`によるガードの実装、パラメータ一覧）は `e2e/README.md` を参照。

## 実データに触れないはずの作業を行う場合のmtime照合

サンドボックスガード自体の検証作業など、「実データに触れないはず」という前提で作業する場合は、作業開始前後で対象パスの `LastWriteTime` を照合する。

- 対象: `target/{debug,release}/GrayBrowser/app.db`・`app.db-wal`・`app.db-shm`、`WhiteBrowser_test/*.wb`
- **照合はセッション内で完結する前後比較として行う。** 作業開始直前に取得し、作業終了直後に比較する。セッションをまたいだ比較（前回セッション終了時点の値との比較等）を事故の根拠にしない。実データ保有者は自分のアプリを日常的に使うため、間に正常な使用が挟まれば必ず「変化あり」になる
- **変化を見つけた場合、調査を始める前に「その間にアプリを使ったか」を人間に確認する。** この確認を飛ばして事故と誤判定した実例がある（`app.db-wal`の増加を検知しエージェントの操作ミスと推定したが、実際は実データ保有者本人がアプリを操作した結果だった）
