# GrayBrowser リデザイン — トークン差し替え用 CSS

`src/App.css` の `:root` を案ごとに置き換えます。ライトが既定値、ダークは `prefers-color-scheme` で上書きします。

## 案A · Fluent / Mica

Windows 11 標準アプリと地続きの見た目。Mica の地に、角丸 8px のレイヤー面を 1 枚載せて階層を作る。

- 長所: OS と馴染み、長時間見ても違和感が少ない。ライト/ダークの切り替えも自然。
- 短所: 個性は弱め。行をカード化するので、1 画面あたりの行数が約 3% 減る。
- DOM構造の変更:
  - タグバーを「絞り込み中バー」に置き換える。選択中のフォルダ・タグ・★をチップで並べ、×で個別に解除、「すべて解除」で一括解除する。何も絞り込んでいないときはバーごと非表示にする（その分だけ一覧が広がる）。
  - 本物の Mica にするなら tauri.conf の windows に transparent: true と windowEffects: { effects: ["mica"] } を設定し、--page-bg を半透明（例: rgba(28,31,38,.72)）にする。設定を変えずにトークンだけ差し替える場合は、モックのように不透明色で近似する。
  - 一覧エリアをレイヤー面として包む要素（左上だけ角丸のラッパー div）を 1 つ追加する。
  - 左ペイン見出しに SVG アイコンを入れる（既存の絵文字や色付きアイコンを置き換える）。

```css
/* 案A · Fluent / Mica */
:root {
  color-scheme: light dark;
  font-family: "Segoe UI Variable Text", "Segoe UI", "Yu Gothic UI", "Meiryo UI", sans-serif;
  font-size: 13px;
  --font-xs: 11px;
  --font-sm: 12px;
  --font-md: 13px;
  --font-lg: 15px;
  --radius-sm: 4px;
  --radius-md: 8px;
  --radius-pill: 999px;
  --page-bg: #D9DADD;
  --surface: #E1E2E5;
  --surface-2: #E8E9EC;
  --text: #1B1B1B;
  --text-muted: #4F535A;
  --text-subtle: #5B5F66;
  --row-border: #C7CAD0;
  --border-strong: #A9ADB5;
  --accent: #00508D;
  --accent-fg: #FFFFFF;
  --row-selected-bg: #C6D8E9;
  --hover-bg: #D4D6DA;
  --chip-bg: #CFD2D8;
  --chip-text: #26292E;
  --star: #9E5F00;
  --danger: #C42B1C;
  --warn: #8A5100;
  --success: #0F7B0F;
  --scrollbar-thumb: #979BA3;
  --scrim: rgba(0,0,0,0.30);
  --focus-ring: 0 0 0 2px var(--accent);
}
@media (prefers-color-scheme: dark) {
  :root {
    --page-bg: #1C1F26;
    --surface: #252932;
    --surface-2: #2D323C;
    --text: #F3F4F6;
    --text-muted: #B4BAC6;
    --text-subtle: #A0A7B3;
    --row-border: #353A45;
    --border-strong: #4A505C;
    --accent: #60CDFF;
    --accent-fg: #00253A;
    --row-selected-bg: #1F3A4C;
    --hover-bg: #2C313B;
    --chip-bg: #353B47;
    --chip-text: #E3E7EE;
    --star: #FFC83D;
    --danger: #FF99A4;
    --warn: #F7C948;
    --success: #6CCB5F;
    --scrollbar-thumb: #5A606C;
    --scrim: rgba(0,0,0,0.55);
  }
}
```

## 案B · ダーク・シネマ

UI はほぼ黒に沈め、サムネイルだけを浮かび上がらせる。情報パネルは一段暗くし、行にカーソルを合わせたときに読む前提。

- 長所: サムネイルが最も見やすく、作品を目で探す作業に強い。暗所での長時間利用に最適。
- 短所: メタ情報が控えめなので、数値で比較・整理する作業では読み取りが一手遅れる。ライト版は個性が出にくい。
- DOM構造の変更:
  - タグバーを「絞り込み中バー」に置き換える。選択中のフォルダ・タグ・★をチップで並べ、×で個別に解除、「すべて解除」で一括解除する。何も絞り込んでいないときはバーごと非表示にする（その分だけ一覧が広がる）。
  - トークン差し替えだけで成立する。情報パネルの減光は .row:hover / .row.selected .info の色指定を CSS に追加するだけで済む。
  - （任意）タグバーを左寄せの 1 行にして、溢れた分は「+N」チップに畳む。

```css
/* 案B · ダーク・シネマ */
:root {
  color-scheme: light dark;
  font-family: "Segoe UI Variable Text", "Segoe UI", "Yu Gothic UI", "Meiryo UI", sans-serif;
  font-size: 13px;
  --font-xs: 11px;
  --font-sm: 12px;
  --font-md: 13px;
  --font-lg: 15px;
  --radius-sm: 4px;
  --radius-md: 6px;
  --radius-pill: 999px;
  --page-bg: #DAD7D2;
  --surface: #E2DFDA;
  --surface-2: #E9E6E1;
  --text: #18181B;
  --text-muted: #47474F;
  --text-subtle: #56565E;
  --row-border: #CBC7C0;
  --border-strong: #B1ACA3;
  --accent: #8A4106;
  --accent-fg: #FFFFFF;
  --row-selected-bg: #E5D3B5;
  --hover-bg: #D6D3CD;
  --chip-bg: #D1CDC6;
  --chip-text: #2F2F35;
  --star: #A85A05;
  --danger: #B91C1C;
  --warn: #A8380A;
  --success: #15803D;
  --scrollbar-thumb: #ACA69C;
  --scrim: rgba(24,24,27,0.35);
  --focus-ring: 0 0 0 2px var(--accent);
}
@media (prefers-color-scheme: dark) {
  :root {
    --page-bg: #0B0B0D;
    --surface: #111114;
    --surface-2: #18181C;
    --text: #ECECEE;
    --text-muted: #A1A1AA;
    --text-subtle: #8E8E97;
    --row-border: #1F1F24;
    --border-strong: #33333B;
    --accent: #F5B544;
    --accent-fg: #1A1203;
    --row-selected-bg: #241D10;
    --hover-bg: #17171B;
    --chip-bg: #1F1F24;
    --chip-text: #D4D4D8;
    --star: #F5B544;
    --danger: #F87171;
    --warn: #FB923C;
    --success: #4ADE80;
    --scrollbar-thumb: #3F3F46;
    --scrim: rgba(0,0,0,0.70);
  }
}
```

## 案C · ネイビー・コンソール（高密度）

参考の Gentelella の質感（紺の地、一段明るいカード、ティール、ピル型ステータス）を、作業ツール向けの密度に詰めた案。

- 長所: 1 画面に載る情報量が最大。数値・タグでの整理作業がいちばん速い。
- 短所: 基本 12px なので、高 DPI でない環境や年配の利用者には小さい。紺の地は好みが分かれる。
- DOM構造の変更:
  - タグバーを「絞り込み中バー」に置き換える。選択中のフォルダ・タグ・★をチップで並べ、×で個別に解除、「すべて解除」で一括解除する。何も絞り込んでいないときはバーごと非表示にする（その分だけ一覧が広がる）。
  - 一覧の上にカードヘッダー（絞り込み後の件数と全体件数）を 1 行追加する。
  - 行のレイアウトを「左: ファイル名＋サムネイル列 / 右: 情報パネル（行の上端から）」の2カラムに組み替える。情報パネルがファイル名の行から始まるので、行の高さが約14px縮む。
  - 行頭にチェックボックスを追加する（複数選択を見えるようにするため。Ctrl/Shift 選択と併用）。
  - 左ペインの各項目の右端に、件数のピルを追加する（データは既存のものを使う）。

```css
/* 案C · ネイビー・コンソール（高密度） */
:root {
  color-scheme: light dark;
  font-family: "Segoe UI Variable Text", "Segoe UI", "Yu Gothic UI", "Meiryo UI", sans-serif;
  font-size: 12px;
  --font-xs: 10.5px;
  --font-sm: 11.5px;
  --font-md: 12px;
  --font-lg: 14px;
  --radius-sm: 3px;
  --radius-md: 6px;
  --radius-pill: 999px;
  --page-bg: #D6DBE4;
  --surface: #DFE3EA;
  --surface-2: #E6E9EF;
  --text: #0F1A2B;
  --text-muted: #42506A;
  --text-subtle: #505D74;
  --row-border: #C4CBD8;
  --border-strong: #A8B2C4;
  --accent: #06624F;
  --accent-fg: #FFFFFF;
  --row-selected-bg: #C0DED6;
  --hover-bg: #D2D8E2;
  --chip-bg: #CDD4E0;
  --chip-text: #1C2B45;
  --star: #9C6302;
  --danger: #C0263A;
  --warn: #8C5000;
  --success: #13804F;
  --scrollbar-thumb: #9CA7B9;
  --scrim: rgba(15,26,43,0.40);
  --focus-ring: 0 0 0 2px var(--accent);
}
@media (prefers-color-scheme: dark) {
  :root {
    --page-bg: #0D1526;
    --surface: #131D31;
    --surface-2: #18243B;
    --text: #E6EBF3;
    --text-muted: #9AA8BF;
    --text-subtle: #8C9AB2;
    --row-border: #22304A;
    --border-strong: #2E3F5E;
    --accent: #26C6A6;
    --accent-fg: #04201A;
    --row-selected-bg: #123236;
    --hover-bg: #17233A;
    --chip-bg: #1E2C46;
    --chip-text: #C9D4E5;
    --star: #F0B44C;
    --danger: #F2707F;
    --warn: #F0B44C;
    --success: #3DD68C;
    --scrollbar-thumb: #33445F;
    --scrim: rgba(5,10,20,0.65);
  }
}
```

## 共通で追加する CSS（どの案でも必要）

```css
/* 横スクロールバーをテーマに合わせる（WebView2 = Chromium） */
.thumb-strip { scrollbar-width: thin; scrollbar-color: var(--scrollbar-thumb) transparent; }
.thumb-strip::-webkit-scrollbar { height: 6px; }
.thumb-strip::-webkit-scrollbar-thumb { background: var(--scrollbar-thumb); border-radius: 3px; }
.thumb-strip::-webkit-scrollbar-track { background: transparent; }

:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
@media (prefers-reduced-motion: reduce) { * { transition: none !important; animation: none !important; } }
```
