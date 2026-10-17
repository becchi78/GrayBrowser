import type { DuplicateGroup, DuplicateGroupMember } from "../types.ts";

// 「ファイルも削除」を無効にする理由。有効なら null。
// バックエンドの判定と同じ規則で、"online" ちょうどのものだけをオンラインと
// 数える。想定外の status は「迷ったら無効側」に倒すためオフライン扱いにする。
// 内容の一致が確認済みの種類だけを許可し、未知の kind も無効側に倒す
// （サイズやハッシュの一致だけでは、同じ内容のコピーとは言い切れないため）。
// 画面の無効化は親切のためで、最終的な拒否はバックエンドが行う。
export function fileDeleteDisabledReason(
  member: DuplicateGroupMember,
  group: DuplicateGroup,
): string | null {
  if (member.status !== "online") {
    return "オフラインのため削除できません";
  }
  if (group.kind !== "quick_hash_confirmed" && group.kind !== "path_collision_confirmed") {
    return "内容の一致が確認できていないため削除できません";
  }
  const onlineCount = group.members.filter((m) => m.status === "online").length;
  if (onlineCount <= 1) {
    return "このグループで到達できる最後の1件のため削除できません";
  }
  return null;
}
