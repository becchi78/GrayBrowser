// fileDeleteDisabledReason のユニットテスト。
// src/lib/paths.test.ts と同じパターン（package.jsonのtest:unit参照）。
import assert from "node:assert/strict";
import { test } from "node:test";
import { fileDeleteDisabledReason } from "./duplicateDelete.ts";
import type { DuplicateGroup, DuplicateGroupMember } from "../types.ts";

const LAST = "このグループで到達できる最後の1件のため削除できません";
const OFFLINE = "オフラインのため削除できません";

function member(id: string, status: string): DuplicateGroupMember {
  return {
    video_id: id,
    file_path: `/v/${id}.mp4`,
    file_name: `${id}.mp4`,
    file_size: 1000,
    status: status as DuplicateGroupMember["status"],
    created_at: "2026-01-01 00:00:00",
    duration: 60,
    width: 1920,
    height: 1080,
  };
}

function group(members: DuplicateGroupMember[]): DuplicateGroup {
  return { kind: "quick_hash_confirmed", members };
}

test("他にオンラインのメンバーがいれば有効", () => {
  const a = member("a", "online");
  assert.equal(fileDeleteDisabledReason(a, group([a, member("b", "online")])), null);
});

test("オフラインのメンバーは無効", () => {
  const a = member("a", "offline");
  assert.equal(
    fileDeleteDisabledReason(a, group([a, member("b", "online"), member("c", "online")])),
    OFFLINE,
  );
});

test("他がオフラインだけなら最後の1件として無効", () => {
  const a = member("a", "online");
  assert.equal(fileDeleteDisabledReason(a, group([a, member("b", "offline")])), LAST);
});

test("1件だけのグループは無効", () => {
  const a = member("a", "online");
  assert.equal(fileDeleteDisabledReason(a, group([a])), LAST);
});

test("オンライン2件とオフライン1件なら、オンラインの側は有効", () => {
  const a = member("a", "online");
  assert.equal(
    fileDeleteDisabledReason(a, group([a, member("b", "online"), member("c", "offline")])),
    null,
  );
});

test("status が想定外の値なら無効側に倒す", () => {
  const a = member("a", "ONLINE");
  assert.equal(
    fileDeleteDisabledReason(a, group([a, member("b", "online"), member("c", "online")])),
    OFFLINE,
  );
  const b = member("b", "online");
  assert.equal(fileDeleteDisabledReason(b, group([member("a", "ONLINE"), b])), LAST);
});
