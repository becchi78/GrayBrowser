import type { DuplicateGroupMember } from "../types";

export interface DuplicateDiff {
  file_size: boolean;
  duration: boolean;
  resolution: boolean;
}

const NO_DIFF: DuplicateDiff = {
  file_size: false,
  duration: false,
  resolution: false,
};

// null を含む項目は「同じ」とは言えないため不一致扱いにする。オフライン等で
// 値が取れていないメンバーを、他と一致しているように見せないため。
function differs<T extends number | null>(values: T[]): boolean {
  if (values.some((v) => v === null)) return true;
  return values.some((v) => v !== values[0]);
}

// 幅と高さは組で1つの値として比べる。片方だけ一致しても同じ解像度とは言えず、
// どちらかが null なら表示も「不明」になるため不一致扱いにそろえる。
function resolutionDiffers(members: DuplicateGroupMember[]): boolean {
  if (members.some((m) => m.width === null || m.height === null)) return true;
  return members.some((m) => m.width !== members[0].width || m.height !== members[0].height);
}

/**
 * グループ内で値が全メンバー同一でない項目を返す。メンバーが1件以下の
 * グループには比べる相手がいないため、何も強調しない。
 */
export function computeDuplicateDiff(members: DuplicateGroupMember[]): DuplicateDiff {
  if (members.length < 2) return { ...NO_DIFF };
  return {
    file_size: differs(members.map((m) => m.file_size)),
    duration: differs(members.map((m) => m.duration)),
    resolution: resolutionDiffers(members),
  };
}
