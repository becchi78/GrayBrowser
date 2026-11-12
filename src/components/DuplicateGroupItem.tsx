import { computeDuplicateDiff } from "../lib/duplicateDiff";
import { fileDeleteDisabledReason } from "../lib/duplicateDelete";
import type { DuplicateGroup, DuplicateGroupKind, DuplicateGroupMember } from "../types";
import { DuplicateGroupMemberRow } from "./DuplicateGroupMemberRow";

const KIND_LABELS: Record<DuplicateGroupKind, string> = {
  quick_hash_confirmed: "内容一致（確定）",
  path_collision_confirmed: "パス衝突・内容確認済み",
  path_collision_unconfirmed: "パス衝突・内容未確認",
  quick_hash_candidate_unverified: "候補・内容未確認",
};

const UNVERIFIED_CANDIDATE_NOTE = "NAS に接続できないため、内容が同じか確認できません";

// 重複候補のグループ1つ。種別のラベルを出し、メンバーを並べ替えて差分を計算する。
export function DuplicateGroupItem({
  group,
  busy,
  fileDeleteErrors,
  onDelete,
  onDeleteFile,
}: {
  group: DuplicateGroup;
  busy: boolean;
  fileDeleteErrors: Record<string, string>;
  onDelete: (member: DuplicateGroupMember) => void;
  onDeleteFile: (member: DuplicateGroupMember) => void;
}) {
  // 登録日時の古い順に並べる。先に登録された方が「元」である可能性が高く、
  // 毎回同じ順序で並ぶので見比べるときに位置が入れ替わらない。
  // created_at は固定幅の文字列なので辞書順比較でよい。
  const members = [...group.members].sort((a, b) =>
    a.created_at < b.created_at ? -1 : a.created_at > b.created_at ? 1 : 0,
  );
  const diff = computeDuplicateDiff(members);

  return (
    <li className="duplicate-group">
      <div className="duplicate-group-header">
        <span className="duplicate-group-kind">{KIND_LABELS[group.kind]}</span>
        {group.kind === "quick_hash_candidate_unverified" && (
          <span className="duplicate-group-kind-note">{UNVERIFIED_CANDIDATE_NOTE}</span>
        )}
      </div>
      <ul className="duplicate-group-members">
        {members.map((member) => (
          <DuplicateGroupMemberRow
            key={member.video_id}
            member={member}
            diff={diff}
            fileDeleteDisabledReason={fileDeleteDisabledReason(member, group)}
            busy={busy}
            deleteFileError={fileDeleteErrors[member.video_id] ?? null}
            onDelete={() => onDelete(member)}
            onDeleteFile={() => onDeleteFile(member)}
          />
        ))}
      </ul>
    </li>
  );
}
