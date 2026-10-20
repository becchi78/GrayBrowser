---
name: release-publish
description: GrayBrowserのインストーラーをビルドし、GitHub ReleasesにWindows用exeを公開する手順。バージョンを上げてリリースする、インストーラーを配布する、GitHub Releasesにアセットを添付する際に参照する。
---

# GrayBrowser リリース公開ガイド

## 配布形式

Windows向けインストーラーはNSIS形式(`.exe`)をメインで配布する。`tauri.conf.json`の`bundle.targets: "all"`によりMSI(`.msi`)も同時に生成されるが、`installMode: currentUser`(管理者権限不要のユーザー単位インストール)と相性が良いのはNSISのため、Releasesのメイン配布物は`.exe`とする。

## アプリ内アップデートと署名鍵

アプリは起動時(設定でオフ可)と「ヘルプ > 更新の確認...」で、最新のReleaseに添付された`latest.json`(`https://github.com/becchi78/GrayBrowser/releases/latest/download/latest.json`)を読み、新しい版があればインストーラーをダウンロードして適用する。インストーラーは秘密鍵で署名され、アプリに埋め込んだ公開鍵(`src-tauri/tauri.conf.json`の`plugins.updater.pubkey`)で検証できなければ適用されない。

- **秘密鍵の控え**: `C:\Users\bessh\.graybrowser-signing\`(リポジトリ外)。GitHub Secretの`TAURI_SIGNING_PRIVATE_KEY`にも登録済み。秘密鍵はリポジトリ・ログ・チャット・コミットに絶対に出さない。パスワードなしの鍵のため、ワークフローでは`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`を空文字で明示している(未設定だとパスワードの対話入力を待って止まる)。
- **鍵を失う・差し替える場合**: 秘密鍵を失うと、既存の版に埋め込まれた公開鍵で検証できる署名を作れなくなり、**既存の利用者はアプリ内で更新できず、手動で入れ直す必要がある**。再生成する場合は、`npx tauri signer generate -w <出力先>`で新しい鍵ペアを作り、(1)新しい秘密鍵を`gh secret set TAURI_SIGNING_PRIVATE_KEY`で登録、(2)新しい公開鍵を`tauri.conf.json`の`pubkey`へ反映、(3)その版は手動インストールが必要な旨をリリースノートに明記、の順で行う。
- **公開した瞬間に配信が始まる**: `releases/latest`はDraftを除くため、Draftの間は誰にも届かない。Draftを公開(`--draft=false`)した時点で、起動時の確認が有効な全利用者に通知される。問題のある版は公開後の取り消しが難しいので、公開前の確認を省略しない。
- 更新機能を持たない版(v0.8.4以前)の利用者は、この機能が入った最初の版だけ手動でインストールする。

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
   - `npm run tauri build -- --bundles nsis --config src-tauri/tauri.release.conf.json`でNSISインストーラーだけをビルド(MSIは配布しない)。秘密鍵をこのステップにだけ環境変数で渡し、インストーラーの署名ファイル(`GrayBrowser_<version>_x64-setup.exe.sig`)を作る
   - 配布する実行ファイルにデバッグ限定の更新先上書きが残っていないことを確認
   - `.sig`から`latest.json`(`version`・`notes`=CHANGELOGの当該項・`pub_date`・`platforms."windows-x86_64".{signature,url}`)を生成(`.sig`か`latest.json`を作れなければDraftを作らず失敗)
   - `CHANGELOG.md`の当該項・SmartScreenの案内・インストーラーのSHA256からリリースノートを自動生成
   - インストーラーと`latest.json`を添付した**Draft**のReleaseを作成(同じタグのReleaseが既にあれば上書きせず失敗)

   Actionsの実行結果は`gh run list --workflow release.yml`や`gh run watch`で確認する。

5. **Draftを確認して公開**: DraftのReleaseを開き、ノートの体裁(変更点・SmartScreenの節・SHA256の節)と添付ファイル(`GrayBrowser_<version>_x64-setup.exe`と`latest.json`)を確認する。`latest.json`は、`version`がタグと一致すること、`notes`がCHANGELOGの当該項であること、`platforms."windows-x86_64".signature`が空でないこと、`url`がこのReleaseのインストーラーを指していることを見る。可能ならDraftからインストーラーをダウンロードして実機で起動確認する(`sandboxed-verification`スキルの手順に従い、実データを汚さない環境で)。問題なければ、承認を得てから公開する。

   ```bash
   gh release edit v<version> --draft=false
   ```

   問題があった場合は公開せず、Draftとタグを削除して修正し、やり直す(Draftの削除・タグの削除も他者から見える操作のため、承認を得てから行う)。

## Actionsが失敗した場合の手元ビルド(代替手順)

Actionsを直せない・急ぐ場合のみ、手元でビルドして手動でReleaseを作る。

1. ビルド(出力先はワークスペース直下の`target/`)。署名鍵は手元の控え(`C:\Users\bessh\.graybrowser-signing\`)から環境変数に読み込む。鍵の内容は画面・ログに出さない:

   ```bash
   export TAURI_SIGNING_PRIVATE_KEY="$(cat <鍵ファイルのパス>)"
   export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
   npm run tauri build -- --bundles nsis --config src-tauri/tauri.release.conf.json
   ```

   出力: `target/release/bundle/nsis/GrayBrowser_<version>_x64-setup.exe`と、その署名`GrayBrowser_<version>_x64-setup.exe.sig`

2. リリースノートと`latest.json`を、リポジトリ外の一時ディレクトリへ生成する(誤ってコミットしないため。公開後に削除する)。SHA256はインストーラーから自動で算出される。

   ```bash
   node scripts/release/release-notes.mjs v<version> target/release/bundle/nsis/GrayBrowser_<version>_x64-setup.exe > <一時ディレクトリ>/notes.md
   node scripts/release/update-manifest.mjs v<version> target/release/bundle/nsis/GrayBrowser_<version>_x64-setup.exe.sig > <一時ディレクトリ>/latest.json
   ```

3. タグ(未pushなら手順4)の後、Draftで作成して確認してから公開する。

   ```bash
   gh release create v<version> "target/release/bundle/nsis/GrayBrowser_<version>_x64-setup.exe" "<一時ディレクトリ>/latest.json" \
     --draft --verify-tag --title "v<version>" \
     --notes-file <一時ディレクトリ>/notes.md
   ```

   タグのpushでActionsが既にDraftを作っていた場合は重複させない(先にそのDraftを削除する)。

## 注意事項

- タグのpush・リリースの公開はいずれも他者から見える・取り消しにくい操作である。実行前に必ずユーザーに内容(バージョン番号・リリースノート)を提示し、承認を得てから行う。タグをpushした時点でActionsが走るため、`CHANGELOG.md`の内容(=リリースノートの変更点)はタグ前に確定させる。
- リリースノートの変更点は`CHANGELOG.md`が一次情報(`###`見出しは`##`に上げて転記される)。利用者視点で簡潔に書く(内部リファクタ等、利用者に関係ない変更は割愛してよい)。
- `main`ブランチの状態からタグを打つ(featureブランチの途中状態でリリースしない)。
