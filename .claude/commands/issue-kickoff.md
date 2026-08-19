---
description: 指定したIssueを取得し、Issue読み込み→関連設計書確認→Plan提示を実行する。承認前に実装には入らない。
argument-hint: [Issue番号。例: 61]
---

Issue #`$1` に基づく改善タスクの着手手順を開始する。`.claude/skills/github-workflow/SKILL.md` に従うこと。

以下を順に行う（この段階では実装しない）。

1. **読み込み**:
   - `gh issue view $1 --comments` で本文と全コメントを読む。
   - ラベル（カテゴリ: `bug`/`ux`/`test-infra`/`perf`/`documentation`、着手可否: `ready`/`blocked`）を確認する。`blocked` が付いている、または `ready` が付いていない場合は、その旨を明示してユーザーに続行してよいか確認する。
   - Issueが参照指定している設計書（`doc/`配下）・関連資料を読む。
   - `CLAUDE.md`

2. **現状確認**:
   - `git status` と `git log --oneline -5` で現在のブランチ・直近コミットを確認する。
   - ブランチ名 `feature/$1-<短い説明>` を決める（まだ作らない。計画に含めて提示する）。

3. **Plan 提示**（承認ゲート。ここで止まる）:
   - Issueのスコープ・DoDを解消する実装計画をPlan modeで提示する。必ず含める:
     - 実装順序（依存関係を守る）
     - DBスキーマ変更が伴う場合はマイグレーション方針（`schema_version`）
     - OS依存アダプタ層とビジネスロジックの分離境界（該当する場合）
     - CI（`ci.yml`）との整合（該当する場合）
     - DoD各項目をどう満たすか
     - `implementer`への委譲単位（実装内容ごとに、目的・スコープ・完了条件を分ける）
   - Issueのスコープ外に踏み出していないことを明示する。
   - Issueの「要判断事項」欄に記載がある場合、あるいは新たに判断が必要な点が見つかった場合は、推奨案と理由を添えて提示する。

**承認を得るまで実装に入らないこと。** 計画提示後、人間の承認を待つ。承認後は、`.claude/skills/github-workflow/SKILL.md`のIssue起点の改善タスクの進め方（ブランチ作成→`implementer`委譲→`reviewer`検収→PR本文に`Closes #$1`→PR提出で停止・マージしない）に進む。
