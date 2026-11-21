# GrayBrowser ブランド素材

## icon/
| ファイル | 用途 |
|---|---|
| `graybrowser-icon.svg` | 元データ（ベクター）。サイズを変えるときはここから書き出す |
| `icon-1024.png` | Tauri のアイコン生成の元画像 |
| `graybrowser.ico` | Windows 用（16/24/32/48/64/128/256px を 1 ファイルに格納） |
| `icon-16.png` 〜 `icon-512.png` | 各サイズの PNG（背景透過） |

Tauri のアイコン一式は、リポジトリのルートで次を実行すると `src-tauri/icons/` に生成される（既存ファイルは上書きされる）。

```
npx tauri icon path/to/icon-1024.png
```

## banner/
| ファイル | サイズ |
|---|---|
| `banner-dark.png` / `banner-dark@2x.png` | 1200×280 / 2400×560（ダーク地） |
| `banner-light.png` / `banner-light@2x.png` | 1200×280 / 2400×560（ライト地） |

GitHub の README で OS のテーマに合わせて切り替える例：

```html
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="image/banner/banner-dark@2x.png">
  <img alt="GrayBrowser" src="image/banner/banner-light@2x.png" width="600">
</picture>
```

書体：Space Grotesk（Medium 500 / Bold 700、SIL Open Font License）
