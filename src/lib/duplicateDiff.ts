import type { DuplicateGroupMember } from "../types";

export interface DuplicateDiff {
  file_size: boolean;
  duration: boolean;
  rating: boolean;
  tags: boolean;
  mtime: boolean;
}

const NO_DIFF: DuplicateDiff = {
  file_size: false,
  duration: false,
  rating: false,
  tags: false,
  mtime: false,
};

// null を含む項目は「同じ」とは言えないため不一致扱いにする。オフライン等で
// 値が取れていないメンバーを、他と一致しているように見せないため。
function differs<T extends number | null>(values: T[]): boolean {
  if (values.some((v) => v === null)) return true;
  return values.some((v) => v !== values[0]);
}

// タグはバックエンドが名前順で返すが、比較は順序に依存させない。
function tagsKey(tags: string[]): string {
  return JSON.stringify([...tags].sort());
}

/**
 * グループ内で値が全メンバー同一でない項目を返す。メンバーが1件以下の
 * グループには比べる相手がいないため、何も強調しない。
 */
export function computeDuplicateDiff(members: DuplicateGroupMember[]): DuplicateDiff {
  if (members.length < 2) return { ...NO_DIFF };
  const tagKeys = members.map((m) => tagsKey(m.tags));
  return {
    file_size: differs(members.map((m) => m.file_size)),
    duration: differs(members.map((m) => m.duration)),
    rating: differs(members.map((m) => m.rating)),
    tags: tagKeys.some((k) => k !== tagKeys[0]),
    mtime: differs(members.map((m) => m.mtime)),
  };
}
