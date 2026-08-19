# CLAUDE.md

このファイルはClaude Code向けの指示です。GrayBrowserリポジトリで作業する際は、必ずこの内容に従ってください。

GrayBrowserは、大量のローカル動画ファイルを高速にスキャン・カタログ化し、タグ付け・レーティング・検索で整理できる、Windows向けのローカル動画管理デスクトップアプリです。プロダクトの説明・使い方は `README.md` を参照してください。

## 技術スタック

- アプリケーションフレームワーク: Tauri v2（Rust + Web）。フロントエンドは React + TypeScript + Vite
- データベース: SQLite3（`rusqlite` ＋ 読み取りは `r2d2`/`r2d2_sqlite` プール、WALモード。`sqlx` は使わない）
- プロジェクト構成: Cargoワークスペース（`crates/gb-core` ＝ OS非依存ロジック、`src-tauri` ＝ Windows/Tauri依存層）
- 動画解析: FFmpeg / FFprobe
- ハッシュ計算: xxHash64（quick_hash）／ BLAKE3（full_hash）
- サムネイル画像形式: WebP（低品質設定）
- CI/CD: GitHub Actions（`windows-latest`ランナー）

## 開発環境

開発はWindows実機（またはWindows VM）に統一する。実装・ビルド・実行確認・デバッグをすべて同一環境で行う。

## コーディング規約・設計方針

- **OS依存ロジックの分離**: ビジネスロジック（ハッシュ計算、機種依存文字バリデーション、パスマッチング等）はOS非依存の純粋関数として `crates/gb-core` に実装し、`#[cfg(windows)]` を含めない。Windows API依存処理はtraitを介した薄いアダプタ層（`src-tauri/src/adapters/`）に閉じ込める。`gb-core` に `tauri`/`rusqlite`/`notify`/`windows-sys` を依存させない。これはテストの高速化・決定性確保のための方針であり、実行環境の制約とは無関係に維持する。
- **trait+adapter+fakeパターン**: 新規にOS/外部依存を足すときは、`crates/gb-core/src/ports/` にtraitを定義し、`src-tauri/src/adapters/` に実装し、`crates/gb-core/src/testing/`（Cargo feature `testing` 限定）にフェイクを置く。コマンド層は「フェイク注入可能な薄皮関数 + `#[tauri::command]` ラッパー」に分離し、実Tauriランタイム無しにunit test可能にする。

DBスキーマ・ID設計・参照整合性の規約は `db-schema-change` スキル、サムネイル・機種依存文字ファイル名・外部プロセス呼び出しの規約は `media-ingestion` スキルを参照。

## コメントの書き方

- コメントには非自明な設計判断・トレードオフ・制約の「なぜ」を書く。コードを読む瞬間に必要な情報であり、git logや別文書を辿らせるのは読み手の負担になる。
- Issue/PR番号・Phase/段階名・要件番号・設計書の節番号など、「いつ・誰が・どの管理単位で」実装したかという経緯情報はコメントに書かない。コメントは更新されずに腐りやすく、外部の読者(OSS公開後は特に)には社内の管理番号は文脈不明でリンクとしても機能しない。この種の情報はコミットメッセージ・PR説明に書く。

## ビルド・テストコマンド

Cargoワークスペース（`src-tauri` ＋ `crates/gb-core`）構成での確定コマンドです。lint/test は `gb-core` を対象から漏らさないよう `--workspace` を付ける。

```bash
# Lint / Format
cargo fmt --check --all
cargo clippy --all-targets --workspace -- -D warnings
npm run lint
npm run typecheck

# Unit test
cargo test --all-features --workspace

# 依存クレートの脆弱性チェック
cargo audit --manifest-path src-tauri/Cargo.toml

# ローカル開発（実際のウィンドウ・実際のTauri APIでホットリロード）
cargo tauri dev

# ビルド
npm run tauri build

# E2E
npm run test:e2e
```

## Git / GitHub運用

ブランチ運用・コミット/PR規約・Issue起点の改善タスクの進め方・Issueラベルの意味は `github-workflow` スキルを参照。

## Plan承認ゲート

実装に影響する変更（コード・設定・`.claude/`配下の運用ファイル・`CLAUDE.md`自体を含む）に着手する前は、必ずPlan modeで方針を提示し、ユーザーの承認を得てから着手する。ファイルの一読、`git status`/`git log`等の読み取り専用コマンド、Issue内容の確認といった読み取り専用の操作は対象外。

承認されたPlanを実行に移す際は、`github-workflow` スキルに従い `main` 上で直接作業せず、必ずfeatureブランチを作成してから着手する。実装を伴うPlanには、ブランチ作成をPlanの実行方針に明記すること。

## 実装と検収の分離

非自明な実装作業は `implementer` サブエージェントに委譲し、`reviewer` サブエージェントに独立した検収を行わせる。**同一のエージェントに実装と自己検収の両方を兼ねさせない。** 実装役の「できました」を鵜呑みにせず、`reviewer` の指摘(規約違反・テストの妥当性等)を必ず受け取ってから完了と判断する。ファイルの一読や `cargo tree` の確認など、委譲するほどでない軽微な確認は自分で行ってよい。

## 原因調査の分離

バグ報告・テスト失敗・想定外の挙動など、原因が自明でない調査は `investigator` サブエージェントに委譲する。`investigator` は実地確認による根本原因の切り分けに専念し、実装は行わない。原因が実地確認済みになってから `implementer` に実装を依頼する。「環境要因だろう」「タイミングの問題では」といった推測だけで原因を確定させない。

## 利用可能なスラッシュコマンド

- `/issue-file` — 機能追加・不具合修正の依頼内容を整理し、GitHub Issueとして起票する
- `/issue-kickoff <Issue番号>` — 指定したIssueを読み込み、実装計画を提示する（承認前に実装には入らない）
