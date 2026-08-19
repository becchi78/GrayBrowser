---
name: media-ingestion
description: GrayBrowserのファイルスキャン・サムネイル生成・機種依存文字バリデーション・FFmpeg/FFprobe呼び出しに関する規約。動画ファイルの取り込み処理、サムネイル生成、ファイル名バリデーション、外部プロセス呼び出しを実装・変更する際に参照する。
---

# GrayBrowser メディア取り込みガイド

## サムネイル

WebP形式、低品質設定で生成し、`GrayBrowser\thumbnails\[id].webp` に保存する。書き込みは一時ファイル経由のアトミック書き込み（`.webp.tmp` → rename）で行う。

## 機種依存文字を含むファイル名

DBには一切登録しない（ハッシュ計算・サムネイル生成も行わない）。`skipped_files` テーブルに記録し、UIで通知する。回避策（ハードリンク等）は採用しない。

## 外部プロセス呼び出し

FFmpeg/FFprobeは `std::process::Command` の引数配列形式で呼び出し、シェル文字列結合は使わない。
