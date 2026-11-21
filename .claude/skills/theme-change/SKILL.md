---
name: theme-change
description: GrayBrowserのテーマ(見た目の設計。graphite / console / fluent / cinema など)の追加・変更に関する規約と手順。新しいテーマの追加、テーマのトークンや見た目の変更、src/themes/ 配下のCSS・マニフェスト・コントラスト検査を触る際に参照する。
---

# GrayBrowser テーマ変更ガイド

## 方針

- **テーマで変えてよいのは見た目だけ。** 表示する要素(件数ヘッダー・件数ピル・行頭のチェックボックスなど)や操作は、全テーマで同じにする。テーマの指定で部品を隠さない(`display: none`、`visibility: hidden`、`[hidden]` を使わない)。DOM も全テーマ共通。
- **差はトークンの値だけで表す。** 部品のCSS(`src/App.css`)は `[data-theme` を書かず、`var(--...)` のトークンだけを参照する。テーマごとの違いは `src/themes/<テーマ名>.css` のトークンの値に置く。新しいテーマを足すとき、部品のセレクタを探して回らなくて済む形にするための方針。
- **例外は、トークンで本当に表せないときだけ。** その場合に限り、そのテーマのファイルの中にセレクタ上書きを置く。なぜトークンで表せないかをコメントに書き、PR説明に挙げる。ただし現状は例外ゼロで、`scripts/theme-structure.test.mjs` が「各テーマのファイルに、自分のセレクタ以外が無い」ことを検査する。許可されるセレクタは、`:root[data-theme="<そのテーマ>"]` だけ。素の `:root` は、既定テーマ(マニフェストの `defaultTheme`、現状 graphite)のファイルの `:root, :root[data-theme="graphite"]` にだけ許可される(未設定・未知の `data-theme` での既定を兼ねるため)。他のテーマのファイルに素の `:root` を書くと全テーマに効いてしまうので、自分の定義と数えず、検査で落ちる。例外を入れるなら、その検査に許可を足す変更を同じPRで行い、レビューで理由を確認する。
- 仕様の値(`docs/redesign/graybrowser-redesign-tokens.md`)は変えない。仕様に無い色は `_base.css` の導出ブロックで仕様のトークンから作る。

## ファイル構成

- `src/themes.css` — 入口。`@import` で `_base` → 各テーマの順に束ねるだけ(`main.tsx` / `App.tsx` / `App.css` の import は変えない)。
- `src/themes/_base.css` — `color-scheme`、全テーマ共通の導出、構造トークンの既定値。
- `src/themes/<テーマ名>.css` — テーマごとのトークン。既定テーマ(現状 `graphite`)だけは未設定・未知の `data-theme` での既定を兼ね、セレクタが `:root, :root[data-theme="graphite"]` になる。他は `:root[data-theme="<テーマ名>"]`。
- `src/themes/manifest.json` — テーマ名の一覧(メニューの並び順)と、全テーマが定義しなければならない必須トークン・プロパティ。

## トークンを足すとき

1. 部品のCSSに、トークンを参照する宣言を足す。省略しない・既定を決める(その状態が「素の見た目」になるように)。
2. `_base.css` の「構造トークンの既定値」に既定値を書く。
3. **全テーマ**のファイルにそのトークンの値を明示する(既定と同じ値でも書く)。
4. `manifest.json` の `requiredTokens` に足す。
5. `scripts/theme-equivalence.test.mjs` の `NEW_TOKENS` に足し、旧3テーマ(console / fluent / cinema)の期待値の表に、状態ごとの値を足す(見た目を変えない場合は、旧テーマの値が変わらないように置く)。

JS が読むトークン(`--row-height` は仮想スクロールの行の高さ、`--nav-w` は左ペインの初期幅)は、`readLengthToken` で読む。`--nav-w` のように CSS の部品が参照しないものもあるが、必須トークンとして全テーマに値を持たせ、`manifest.json` の `requiredTokens` に入れる(構造検査は CSS での参照の有無を見ないので、これを許す)。値の単位は長さ(px)にし、読めないときのフォールバックは呼び出し側の定数に置く。

枠の幅・角丸・余白のような辺ごとの値は、上 右 下 左 の4値で持ち、0 の辺は描かれない。枠の `border-style` は部品側で常に `solid` にして、幅だけをトークンにする(行の高さ `--row-height` の積算は枠がボックスに入ることを前提にしているので、幅を変えたら積算を見直す)。

## 新しいテーマの追加手順

1. `src/themes/manifest.json` の `themes` に名前を追加する(並びがメニューの並びになる)。
2. `src/themes/<名前>.css` を作る。`manifest.json` の `requiredTokens` と `requiredProperties`(`font-size` と `line-height`)をすべて定義する。他のテーマのファイルを手本にする。
3. `src/themes.css` に `@import "./themes/<名前>.css";` を、マニフェストの順で足す。
4. Rust: `crates/gb-core/src/appearance.rs` の `Theme` に variant を追加し、`ALL` と `as_str` に足す(保存値の文字列は `<名前>`)。
5. `src/lib/appearance.ts` の `THEMES` と `THEME_LABELS` に足す(メニューは `AppMenu` が `THEMES` から項目を作るので、メニュー側の変更は不要)。
6. `index.html` の同期スクリプトの `var themes = [...]` に足す。
7. 検査を実行する。`scripts/theme-equivalence.test.mjs` は、セレクタ方式だった時点の3テーマだけが対象(新しいテーマの値を縛るものではない)なので、新テーマのために期待値の行を足す必要は無い。

```bash
npm run check:contrast     # 全テーマ×ライト/ダークの文字・UIコントラスト(manifestから自動で拾う)
npm run test:unit          # 必須トークンの欠落、テーマ名の一致、[data-theme の置き場所、旧実装との同値
npm run test:component
cargo test --all-features --workspace
```

テーマ名の一致の検査は、マニフェスト・`appearance.ts` の `THEMES`・Rust の `Theme::ALL` と `as_str`・`index.html`・`THEME_LABELS`・`AppMenu` の項目 id・`src/themes/` のファイルを突き合わせる。足し忘れがあれば落ちる。

新テーマの見た目は、全外観(system / light / dark)で目視確認する。コントラスト検査は色の組の AA を測るだけで、レイアウトや行の高さの収まりは測らない。

## 既定テーマを変えるとき

`manifest.json` の `defaultTheme`、素の `:root` を持つファイル(`:root, :root[data-theme="..."]` を新しい既定のファイルへ移し、元のファイルは `:root[data-theme="..."]` だけにする)、`appearance.ts` の `DEFAULT_THEME`、Rust の `Theme` の既定(`ALL[0]`)、`index.html` の `<html data-theme>` を揃える。`defaultTheme` と `DEFAULT_THEME` の一致は構造検査が見る。

## 既存テーマの値を変えるとき

- 色や導出式を変えたら `npm run check:contrast` を必ず実行する。
- 行の寸法(`--row-height` など)を変えたら、情報パネルの高さを実機で測る(各テーマのコメント参照)。仮想スクロールの行の高さの唯一の情報源。
- 見た目を変えない整理のときは、`scripts/theme-equivalence.test.mjs` の期待値は変えない。見た目を変える変更なら、期待値を意図して更新する。
