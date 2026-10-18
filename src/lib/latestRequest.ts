/**
 * 「最後に発行した要求の応答だけを反映する」ためのガード。
 * 要求の開始時に begin() で連番を取り、応答時に isLatest() が false なら破棄する。
 */
export interface LatestOnlyGuard {
  begin(): number;
  isLatest(id: number): boolean;
}

export function createLatestOnlyGuard(): LatestOnlyGuard {
  let latest = 0;
  return {
    begin() {
      latest += 1;
      return latest;
    },
    isLatest(id) {
      return id > 0 && id === latest;
    },
  };
}
