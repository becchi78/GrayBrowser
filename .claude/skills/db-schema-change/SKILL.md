---
name: db-schema-change
description: GrayBrowserのDBスキーマ変更（マイグレーション追加）の具体的な手順。schema_versionの扱い、マイグレーションの追加場所、参照整合性・gb-core純度の確認方法を扱う。テーブル・カラムの追加、マイグレーションの新規作成、schema_versionを伴う変更を行う際に参照する。
---

# GrayBrowser DBスキーマ変更ガイド

## マイグレーションの構造

- 適用順序を決める純粋ロジックは `crates/gb-core/src/migrations.rs`（`Migration` 構造体・`pending_migrations` 関数、OS/DB非依存）
- 実SQL・DB接続への適用は `src-tauri/src/db/migrations.rs`（`rusqlite::Connection` に対して実行するアダプタ層）
- 適用済みバージョンはDB内の `schema_version` テーブル（`version` / `applied_at`）で管理される。起動時に現在バージョンと定義済みマイグレーションを突き合わせ、未適用分を順に適用する

## ID設計・参照整合性の規約

- **ID設計**: `videos.id` はUUID v4。ハッシュ値（`quick_hash`/`full_hash`）は別カラムとして分離し、主キーには使わない（重複ファイル検出との整合性のため）
- **参照整合性**: `PRAGMA foreign_keys=OFF` 方針のため、`video_tags` 等の参照整合性はDB層で強制されない。書き込み時はアプリ層（単一writerロック内の1トランザクション）で孤児レコードを作らないよう担保する

## 新しいマイグレーションを追加する手順

1. `src-tauri/src/db/migrations.rs` の一覧に、新しい `version`（現行の最大値+1）・`description`・`sql` を持つマイグレーションを追加する
2. 統合テストで、旧スキーマ（マイグレーション適用前）から新スキーマへの適用結果を検証する（`schema_version` が更新されること、新しいテーブル/カラムが期待通りであること）
3. `cargo tree -p gb-core --all-features` で、スキーマ変更に伴う実装が `gb-core` にDB/OS依存を混入させていないか確認する
4. 参照整合性に関わる変更（新しい外部キー相当のカラム等）は、`PRAGMA foreign_keys=OFF` 方針のためDB層では強制されない。孤児レコードが生じないことをアプリ層のテスト（`LEFT JOIN ... WHERE parent.id IS NULL` で孤児ゼロ確認等）で検証する

## 注意

既存マイグレーションの `sql` を後から書き換えない（適用済み環境との整合が取れなくなる）。修正が必要な場合は新しいマイグレーションを追加する。
