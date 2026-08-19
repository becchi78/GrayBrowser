---
name: performance-investigation
description: GrayBrowserで性能・インデックス・クエリ速度に関する主張をする際の検証方法。EXPLAIN QUERY PLANによる実測、search_performance_bench.rsの使い方、ローカル/合成データの結果を実環境の証明として扱わない原則を扱う。性能問題を調査する、インデックス追加を検討する、クエリを最適化する際に参照する。
---

# GrayBrowser 性能調査ガイド

## 原則: 推論ではなく実測で判断する

- 性能・インデックスに関する主張は `EXPLAIN QUERY PLAN` 等の実測に基づかせる。「速いはず」という推論だけでインデックスや最適化を追加しない
- インデックスを追加する場合、実際にそのインデックスが使われているか `EXPLAIN QUERY PLAN` で確認する。使われていなければ採用しない
- ローカルでの成功・体感速度を、実環境（本番相当のビルド種別・データ量）での速度の証明として扱わない

## ベンチマークの実行

`search_performance_bench.rs`（`list_videos_filtered` クエリの性能測定）は `#[ignore]` 付きでreleaseビルド専用。通常の `cargo test --all-features --workspace` では実行されない。

```bash
cargo test --release --test search_performance_bench -- --ignored --nocapture
```

- **必ず `--release` で実行する。** debugビルドの数値は実際にリリースされるアプリを代表しない
- 10,000 / 50,000 / 100,000件のデータセットで、フィルタなし全件ブラウズ・短い検索語・長い検索語・タグ絞り込み併用・各ソート条件を測定する

## 測定条件の明記

- 実データ規模（数千件程度）と大規模ライブラリ（5万〜10万件）とで結論が変わりうる（例: 全件ブラウズは10万件で約1.3秒だが数千件では無関係）。どちらの前提で測ったかを報告に明記する
- 合成データ・debugビルドでの数値を額面通り受け取らない。測定条件（ビルド種別・データ分布・実行順序）が実運用を代表しているかを確認する
