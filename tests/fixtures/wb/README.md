# `.wb` テストフィクスチャ

旧WhiteBrowserの `.wb`（SQLite3）ファイルに関するテストフィクスチャ群。詳細な解析結果は [doc/wb-format.md](../../../doc/wb-format.md) を参照。

## `sample_small.wb`（コミット対象）

開発者の実データ `default_20110504.wb`（movie 3072件）から `wb-anonymize-tool`（`crates/wb-anonymize-tool`）で生成した、匿名化済み・movie 50件のフィクスチャ。**実データの値は一切含まれていない。**

- テーブルは `movie` のみ（`movie_id`/`movie_name`/`movie_path`/`tag`/`score`/`hash`/`kana`/`roma`/`file_date`/`regist_date`/`last_date` の11列に絞った縮小スキーマ。パーサが実際に読む列だけを持つ）。`tagbar`/`watch`/`profile`/`findfact`/`history`/`system`/`sysbin` は含めない(移行・テストいずれにも不要で、匿名化して残す個人情報リスク面積を増やすだけのため)
- `movie_path`/`movie_name`: ドライブレター(`T:`/`U:`/`V:`/`W:`)と拡張子は保持、フォルダ名・ファイル名は決定的なダミートークン(`gbseg_<hex>`)に置換
- `tag`: 改行区切り構造を保持したまま各タグをダミートークン(`gbtag_<hex>`)に置換
- `hash`: 小文字8桁hexの形式・一意性を保持したまま決定的に置換(衝突は自動的に解消)
- `kana`/`roma`: ダミートークン(`gbtxt_<hex>`)に置換
- `score`/`file_date`/`regist_date`/`last_date`: 実データのまま(個人情報ではなく構造的な値のため無変更)

含まれる構造的性質(生成時に確認済み、`crates/gb-core/src/wb_sampling.rs` が保証):

- 50件
- `hash` が空の行を1件含む(実データにある既知ギャップの再現)
- `score` は 0/1〜5/6以上(クランプ対象、20を含む)の全帯にまたがる
- `tag` が空の行・改行区切り複数タグを持つ行の両方を含む
- `movie_path` はドライブレター T:/U:/V:/W: の全てを含む

### 再生成方法

```
cargo run -p wb-anonymize-tool -- tests/fixtures/wb/local/default_20110504.wb tests/fixtures/wb/sample_small.wb
```

実データ `.wb`(`tests/fixtures/wb/local/` 配下、gitignore対象)が手元にある開発者のみ実行できる。入力は読み取り専用でしか開かれず、書き込み先は入力と異なるパスであることを起動時にチェックする(実データの上書きは構造的に不可能)。

生成前に匿名化の網羅性チェック(実データ由来トークンの残存が無いこと、ダミー生成規則に全件マッチすること)が自動実行され、失敗した場合はフィクスチャを書き出さずに終了する。手動で網羅性チェックだけを再実行したい場合:

```
cargo test -p wb-anonymize-tool --test leak_check_against_real_data -- --nocapture
```

このテストは実データが無い環境(CI等)では自動的にskipされる。

### `sample_large.wb`

スコープ外。3072件規模の性能検証が実際に必要になった時点で追加を検討する。実データ・匿名化後版とも約2.6〜3MBで、GitHubのLFS推奨閾値(数十MB〜)を大きく下回るため、追加時もGit LFSは不要と見込む。

## `local/`（コミット対象外）

開発者の実データ `.wb` を置く場所。`.gitignore` の `tests/fixtures/wb/local/` で除外されている。ここに置いたファイルはコミットしないこと。
