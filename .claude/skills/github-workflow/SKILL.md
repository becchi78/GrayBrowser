---
name: github-workflow
description: GrayBrowserリポジトリでのGit/GitHub操作全般のガイド。ブランチ運用・コミット/PR規約・破壊的操作の事前確認・Issue起点の改善タスクの進め方・Issueラベルの意味を扱う。ブランチを切る、コミットする、PRを出す、Issueを起点に作業を始める、GitHub操作全般を行う際に参照する。
---

# GrayBrowser Git/GitHub運用ガイド

## ブランチ運用

- 作業は必ずfeatureブランチで行う。`main` に直接コミット・pushしない。
- ブランチ名: `feature/<Issue番号>-<短い説明>`（例: `feature/61-search-highlight`）。Issueに紐づかない作業は `feature/<短い説明>`。
- 変更は必ずPR経由でマージする。PRのマージ可否はユーザーの判断であり、PR提出までで作業を止める。

## ステージング衛生

- `git add -A` / `git add .` / `git commit -a` は使わない。変更したファイルをパスで明示指定してステージする（`git add path/to/file1 path/to/file2 ...`）。
- 意図した変更だけをステージしたいがファイル内に他の変更も混在する場合は `git add -p` でハンク単位に分離する。
- コミット直前に `git status` を確認し、意図しないファイル（生成物、実データ等）が混入していないか確認する。

## 破壊的操作の事前確認

ファイルの一括削除・上書き、強制フラグ付きコマンド（`-f` / `--force` / `rm -r` / `git reset --hard` / `git clean` 等）を実行する前は、必ず対象と影響範囲を提示し、ユーザーの承認を得てから実行する。

## コミット/PR規約

- Issueに対応する変更のPR本文には `Closes #<N>` を書く。
- push前に、変更した領域に対応する検証（Rust変更なら `cargo build`/`cargo test`、フロントエンド変更なら `npm run lint`/`npm run typecheck`/`npm run test:unit` 等）を行う。
- アプリ本体のコード変更を伴うPRを作成する前に、`app-versioning` スキルの分類基準に従ってバージョンを上げる。そのPRがマージされたら、`release-publish` スキルの手順で実際にリリースする(CHANGELOG記載・タグ作成・GitHub Release公開まで)。バージョン番号を上げるだけで終わらせない。
- PRを提出したら、`sandboxed-verification` スキルの手順（`e2e/run-sandboxed.ps1 -Mode manual`）で実アプリを起動し、変更が実際に動作することを人と一緒に確認する。lint/testなど機械的な検証だけで済ませず、UIやアプリの挙動に関わる変更では目視確認を省略しない。

## Issue起点の改善タスクの進め方

1. `gh issue view <N> --comments` で本文と全コメントを読む。
2. ラベル（カテゴリ: `bug`/`ux`/`test-infra`/`perf`、既定の `documentation`、着手可否: `ready`/`blocked`）を確認する。`blocked` が付いている、または `ready` が付いていない場合は、続行してよいかユーザーに確認する。
3. Issueが参照指定している設計書・関連資料があれば読む。
4. 実装方針（実装順序、DBスキーマ変更を伴う場合はマイグレーション方針、OS依存アダプタ層とビジネスロジックの分離境界など）をPlan modeで提示し、ユーザーの承認を得る。
5. 承認後、`feature/<Issue番号>-<短い説明>` でブランチを作成する。
6. 実装する。非自明な実装は `implementer` サブエージェントに委譲し、`reviewer` サブエージェントで独立に検収する。
7. PR本文に `Closes #<N>` を書いて提出する。マージはユーザーの判断のため、PR提出までで止める。
8. PR提出後、上記「コミット/PR規約」の実アプリ動作確認を行う。

## Issueラベルの意味

- カテゴリ: `bug`（不具合修正）/ `ux`（UI・使い勝手改善）/ `test-infra`（テスト・CI基盤）/ `perf`（性能）/ `documentation`（ドキュメントのみの変更。GitHub既定ラベルを使う）
- 着手可否: `ready`（着手してよい）/ `blocked`（着手不可。理由がIssueに記載される）。相互排他として運用する。

## 並行作業

複数セッション/エージェントで同時に作業する場合は、`git worktree add ../GrayBrowser-<短い説明> <branch>` で物理的にワークツリーを分離する。同一ワークツリーを複数セッションで共有しない。
