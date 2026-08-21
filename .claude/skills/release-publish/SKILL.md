---
name: release-publish
description: GrayBrowserのインストーラーをビルドし、GitHub ReleasesにWindows用exeを公開する手順。バージョンを上げてリリースする、インストーラーを配布する、GitHub Releasesにアセットを添付する際に参照する。
---

# GrayBrowser リリース公開ガイド

## 配布形式

Windows向けインストーラーはNSIS形式(`.exe`)をメインで配布する。`tauri.conf.json`の`bundle.targets: "all"`によりMSI(`.msi`)も同時に生成されるが、`installMode: currentUser`(管理者権限不要のユーザー単位インストール)と相性が良いのはNSISのため、Releasesのメイン配布物は`.exe`とする。

## 手順

1. **バージョンを上げる**(通常は不要): `app-versioning`スキルにより、対象PRのマージ時点で`src-tauri/tauri.conf.json`・`src-tauri/Cargo.toml`・`package.json`のバージョンは既に上がっているはず。3ファイルを確認し、まだ上がっていない場合のみここで更新する。

2. **CHANGELOGを更新する**: `CHANGELOG.md`の先頭(`# Changelog`の直後)に`## [<version>] - <date>`見出しを追加し、カテゴリ(追加/変更/修正/削除)ごとに利用者視点で分かる変更点を箇条書きする。内部リファクタ等、利用者に関係ない変更は割愛してよい。

3. **README.mdを確認・更新する**: 今回のバージョンで加わった変更が、ユーザーが直接体感する機能(「主な機能」等に載せるべきもの)であれば追記する。内部実装のみの変更(ファイル配置の変更等、ユーザーから見えないもの)であれば更新不要。

4. **ビルド**:

   ```bash
   npm run tauri build
   ```

   出力先: `src-tauri/target/release/bundle/nsis/GrayBrowser_<version>_x64-setup.exe`

5. **動作確認**: 生成されたインストーラーを実機で実際にインストールし、起動確認を行う(`sandboxed-verification`スキルの手順に従い、実データを汚さないサンドボックス環境で確認する)。

6. **タグを作成しpush**:

   ```bash
   git tag v<version>
   git push origin v<version>
   ```

7. **リリースを作成しインストーラーを添付**:

   ```bash
   gh release create v<version> "src-tauri/target/release/bundle/nsis/GrayBrowser_<version>_x64-setup.exe" \
     --title "v<version>" \
     --notes "変更内容の要約"
   ```

   `--notes`は`CHANGELOG.md`に書いた当該バージョンの内容を要約・転記する(二重管理を避けるため、CHANGELOGを一次情報とする)。

## 注意事項

- タグのpush・リリースの公開はいずれも他者から見える・取り消しにくい操作である。実行前に必ずユーザーに内容(バージョン番号・リリースノート)を提示し、承認を得てから行う。
- リリースノートには、主要な変更点をユーザー視点で簡潔にまとめる(内部リファクタ等、利用者に関係ない変更は割愛してよい)。
- `main`ブランチの状態からタグを打つ(featureブランチの途中状態でリリースしない)。
