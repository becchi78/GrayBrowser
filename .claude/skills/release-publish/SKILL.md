---
name: release-publish
description: GrayBrowserのインストーラーをビルドし、GitHub ReleasesにWindows用exeを公開する手順。バージョンを上げてリリースする、インストーラーを配布する、GitHub Releasesにアセットを添付する際に参照する。
---

# GrayBrowser リリース公開ガイド

## 配布形式

Windows向けインストーラーはNSIS形式(`.exe`)をメインで配布する。`tauri.conf.json`の`bundle.targets: "all"`によりMSI(`.msi`)も同時に生成されるが、`installMode: currentUser`(管理者権限不要のユーザー単位インストール)と相性が良いのはNSISのため、Releasesのメイン配布物は`.exe`とする。

## 手順

1. **バージョンを上げる**(通常は不要): `app-versioning`スキルの「リリース単位」に従い、`src-tauri/tauri.conf.json`・`src-tauri/Cargo.toml`・`package.json`のバージョンが既に上がっていることを確認する。
   - 個別リリース: 対象PRのマージ時点で上がっているはず。
   - まとめリリース: リリース用PRで更新済みのはず。

   まだ上がっていない場合のみ、PR経由で更新する(バージョン更新だけのPRは、`app-versioning`の「コード変更なしは版を上げない」の例外、すなわちリリース用PRに当たる)。

2. **CHANGELOGを更新する**(`main`へ直接コミットできないため、PR経由。個別リリースでは別PRで実施、まとめリリースではリリース用PRに含める): `CHANGELOG.md`の先頭(`# Changelog`の直後)に`## [<version>] - <date>`見出しを追加し、カテゴリ(追加/変更/修正/削除)ごとに利用者視点で分かる変更点を箇条書きする。内部リファクタ等、利用者に関係ない変更は割愛してよい。

3. **README.mdを確認・更新する**(手順2と同じくPR経由): 今回のバージョンで加わった変更が、ユーザーが直接体感する機能(「主な機能」等に載せるべきもの)であれば追記する。内部実装のみの変更(ファイル配置の変更等、ユーザーから見えないもの)であれば更新不要。

4. **タグをpush**(承認を得てから。`main`の、リリース用PRがマージされた状態で行う):

   ```bash
   git switch main && git pull
   node scripts/release/check-version.mjs v<version>   # 手元でも事前確認できる
   git tag v<version>
   git push origin v<version>
   ```

   `v*`タグのpushで`.github/workflows/release.yml`が起動し、クリーンな`windows-latest`上で次を行う。
   - タグ・`package.json`・`tauri.conf.json`・`Cargo.toml`のバージョンと`CHANGELOG.md`の項の一致を検証(不一致ならビルド前に失敗し、Releaseは作られない)
   - `npm run tauri build -- --bundles nsis`でNSISインストーラーだけをビルド(MSIは配布しない)
   - `CHANGELOG.md`の当該項・SmartScreenの案内・インストーラーのSHA256からリリースノートを自動生成
   - インストーラーを添付した**Draft**のReleaseを作成(同じタグのReleaseが既にあれば上書きせず失敗)

   Actionsの実行結果は`gh run list --workflow release.yml`や`gh run watch`で確認する。

5. **Draftを確認して公開**: DraftのReleaseを開き、ノートの体裁(変更点・SmartScreenの節・SHA256の節)と添付ファイル(`GrayBrowser_<version>_x64-setup.exe`)を確認する。可能ならDraftからインストーラーをダウンロードして実機で起動確認する(`sandboxed-verification`スキルの手順に従い、実データを汚さない環境で)。問題なければ、承認を得てから公開する。

   ```bash
   gh release edit v<version> --draft=false
   ```

   問題があった場合は公開せず、Draftとタグを削除して修正し、やり直す(Draftの削除・タグの削除も他者から見える操作のため、承認を得てから行う)。

## Actionsが失敗した場合の手元ビルド(代替手順)

Actionsを直せない・急ぐ場合のみ、手元でビルドして手動でReleaseを作る。

1. ビルド(出力先はワークスペース直下の`target/`):

   ```bash
   npm run tauri build -- --bundles nsis
   ```

   出力: `target/release/bundle/nsis/GrayBrowser_<version>_x64-setup.exe`

2. リリースノートを、リポジトリ外の一時ディレクトリへ生成する(誤ってコミットしないため。公開後に削除する)。SHA256はインストーラーから自動で算出される。

   ```bash
   node scripts/release/release-notes.mjs v<version> target/release/bundle/nsis/GrayBrowser_<version>_x64-setup.exe > <一時ディレクトリ>/notes.md
   ```

3. タグ(未pushなら手順4)の後、Draftで作成して確認してから公開する。

   ```bash
   gh release create v<version> "target/release/bundle/nsis/GrayBrowser_<version>_x64-setup.exe" \
     --draft --verify-tag --title "v<version>" \
     --notes-file <一時ディレクトリ>/notes.md
   ```

   タグのpushでActionsが既にDraftを作っていた場合は重複させない(先にそのDraftを削除する)。

## 注意事項

- タグのpush・リリースの公開はいずれも他者から見える・取り消しにくい操作である。実行前に必ずユーザーに内容(バージョン番号・リリースノート)を提示し、承認を得てから行う。タグをpushした時点でActionsが走るため、`CHANGELOG.md`の内容(=リリースノートの変更点)はタグ前に確定させる。
- リリースノートの変更点は`CHANGELOG.md`が一次情報(`###`見出しは`##`に上げて転記される)。利用者視点で簡潔に書く(内部リファクタ等、利用者に関係ない変更は割愛してよい)。
- `main`ブランチの状態からタグを打つ(featureブランチの途中状態でリリースしない)。
