---
name: release-publish
description: GrayBrowserのインストーラーをビルドし、GitHub ReleasesにWindows用exeを公開する手順。バージョンを上げてリリースする、インストーラーを配布する、GitHub Releasesにアセットを添付する際に参照する。
---

# GrayBrowser リリース公開ガイド

## 配布形式

Windows向けインストーラーはNSIS形式(`.exe`)をメインで配布する。`tauri.conf.json`の`bundle.targets: "all"`によりMSI(`.msi`)も同時に生成されるが、`installMode: currentUser`(管理者権限不要のユーザー単位インストール)と相性が良いのはNSISのため、Releasesのメイン配布物は`.exe`とする。

## 手順

1. **バージョンを上げる**: `src-tauri/tauri.conf.json`の`version`フィールドを更新する(例: `0.1.0` → `0.2.0`)。`src-tauri/Cargo.toml`の`[package] version`も合わせて更新する。

2. **ビルド**:

   ```bash
   npm run tauri build
   ```

   出力先: `src-tauri/target/release/bundle/nsis/GrayBrowser_<version>_x64-setup.exe`

3. **動作確認**: 生成されたインストーラーを実機で実際にインストールし、起動確認を行う(`sandboxed-verification`スキルの手順に従い、実データを汚さないサンドボックス環境で確認する)。

4. **タグを作成しpush**:

   ```bash
   git tag v<version>
   git push origin v<version>
   ```

5. **リリースを作成しインストーラーを添付**:

   ```bash
   gh release create v<version> "src-tauri/target/release/bundle/nsis/GrayBrowser_<version>_x64-setup.exe" \
     --title "v<version>" \
     --notes "変更内容の要約"
   ```

## 注意事項

- タグのpush・リリースの公開はいずれも他者から見える・取り消しにくい操作である。実行前に必ずユーザーに内容(バージョン番号・リリースノート)を提示し、承認を得てから行う。
- リリースノートには、主要な変更点をユーザー視点で簡潔にまとめる(内部リファクタ等、利用者に関係ない変更は割愛してよい)。
- `main`ブランチの状態からタグを打つ(featureブランチの途中状態でリリースしない)。
