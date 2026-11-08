# Contributing to GrayBrowser

GrayBrowserへの貢献に興味を持っていただきありがとうございます。このドキュメントでは、不具合報告・機能要望・コード貢献の進め方をまとめています。

## 不具合報告・機能要望

[Issue](https://github.com/becchi78/GrayBrowser/issues/new/choose) からお願いします。テンプレートに沿って、背景・再現手順・期待する動作などをできるだけ具体的に記載してください。

## コードで貢献する

このリポジトリへの直接のpush権限は、メンテナ(@becchi78)のみに限定されています。コードの変更はすべて **フォーク → ブランチ作成 → Pull Request** の流れで提出してください。

1. このリポジトリをフォークします。
2. フォーク上でブランチを作成します(例: `feature/short-description`)。
3. 変更を行います。開発環境のセットアップ・ビルド/テストコマンドは [README.md の「開発者向けセットアップ」](README.md#開発者向けセットアップ) を参照してください。
4. コミット前に、変更した領域に対応する検証を行ってください(Rust変更なら `cargo fmt` / `cargo clippy` / `cargo test`、フロントエンド変更なら `npm run lint` / `npm run typecheck` / `npm run test:unit`)。
5. Pull Requestを作成します。何を・なぜ変更したかが分かる説明を書いてください。関連するIssueがあれば本文に `Closes #<Issue番号>` と記載してください。

Pull Requestのレビュー・マージはメンテナが行います。マージには最低1件の承認が必要です。

## コミットメッセージ

「何を」ではなく「なぜ」を意識した簡潔なメッセージを心がけてください。大きな変更は複数の意味のあるコミットに分けることを推奨します。

## テスト

用途に応じて3種類を使い分けます。

- **純粋ロジック**(`src/lib/*.test.ts`): `npm run test:unit`(`node --test`)。新しいテストファイルは `package.json` の `test:unit` の一覧に追加してください。
- **コンポーネント**(`src/**/*.test.tsx`): `npm run test:component`(Vitest + jsdom + Testing Library)。Tauri API は `src/testing/` のフェイクで差し替わり、テストが明示的にモックしていない呼び出しは例外になって失敗します。必要な Context を包む描画ヘルパーは `src/testing/render.tsx` です。
- **実機**(`e2e/specs/*.e2e.mjs`): `e2e/run-sandboxed.ps1` 経由で手動実行します。CI では実行しません。

リファクタリングで動作を変えないことを確かめるときは、先に対象画面の特性テスト(現状の動作をそのまま固定するテスト)を追加してから着手してください。

## その他

コーディング規約・設計方針の詳細は [`CLAUDE.md`](CLAUDE.md) にまとまっています(AIアシスタント向けの記述が中心ですが、人間の貢献者にとっても規約の参考になります)。
