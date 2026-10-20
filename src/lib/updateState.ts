// アップデートダイアログの状態遷移と表示用の計算。Tauri や React に依存しない
// 純関数だけを置き、単体テストで挙動を固定する。

export interface AvailableUpdate {
  version: string;
  currentVersion: string;
  /** リリースノート(CHANGELOG の該当項)。無ければ null。 */
  notes: string | null;
}

export type UpdateState =
  | { kind: "checking" }
  | { kind: "upToDate"; currentVersion: string }
  | { kind: "available"; update: AvailableUpdate }
  // total はサーバーが長さを返さなかったとき null。割合は出さず受信量だけ見せる。
  | { kind: "downloading"; update: AvailableUpdate; downloaded: number; total: number | null }
  // ダウンロードと署名検証が終わり、インストーラーの起動を待っている間。
  | { kind: "installing"; update: AvailableUpdate }
  | { kind: "error"; phase: "check" | "download"; reason: string; update: AvailableUpdate | null };

export type DownloadEvent =
  | { event: "Started"; data: { contentLength?: number } }
  | { event: "Progress"; data: { chunkLength: number } }
  | { event: "Finished" };

export type UpdateAction =
  | { type: "checkStarted" }
  | { type: "checkSucceeded"; result: AvailableUpdate | null; currentVersion: string }
  | { type: "checkFailed"; reason: string }
  | { type: "downloadStarted" }
  | { type: "downloadEvent"; event: DownloadEvent }
  | { type: "downloadFailed"; reason: string };

export const INITIAL_UPDATE_STATE: UpdateState = { kind: "checking" };

/**
 * 状態遷移。許可されない遷移(例: ダウンロード中の再開始、確認中の downloadStarted)は
 * 状態をそのまま返す。ボタンの無効化に頼らず、二重操作をここでも止める。
 */
export function updateReducer(state: UpdateState, action: UpdateAction): UpdateState {
  switch (action.type) {
    case "checkStarted":
      // ダウンロード中・インストーラー起動待ちに確認をやり直させない。
      return state.kind === "downloading" || state.kind === "installing" ? state : { kind: "checking" };
    case "checkSucceeded":
      if (state.kind !== "checking") {
        return state;
      }
      return action.result === null
        ? { kind: "upToDate", currentVersion: action.currentVersion }
        : { kind: "available", update: action.result };
    case "checkFailed":
      if (state.kind !== "checking") {
        return state;
      }
      return { kind: "error", phase: "check", reason: action.reason, update: null };
    case "downloadStarted":
      // 更新の案内が出ている状態(ダウンロード失敗後の再試行を含む)からだけ開始できる。
      if (state.kind === "available") {
        return { kind: "downloading", update: state.update, downloaded: 0, total: null };
      }
      if (state.kind === "error" && state.phase === "download" && state.update !== null) {
        return { kind: "downloading", update: state.update, downloaded: 0, total: null };
      }
      return state;
    case "downloadEvent":
      return applyDownloadEvent(state, action.event);
    case "downloadFailed":
      if (state.kind !== "downloading" && state.kind !== "installing") {
        return state;
      }
      return { kind: "error", phase: "download", reason: action.reason, update: state.update };
  }
}

function applyDownloadEvent(state: UpdateState, event: DownloadEvent): UpdateState {
  if (state.kind !== "downloading") {
    return state;
  }
  switch (event.event) {
    case "Started": {
      const length = event.data.contentLength;
      return { ...state, total: length !== undefined && length > 0 ? length : null };
    }
    case "Progress":
      return { ...state, downloaded: state.downloaded + Math.max(0, event.data.chunkLength) };
    case "Finished":
      return { kind: "installing", update: state.update };
  }
}

/** 0〜100 の整数。合計が不明、または 0 以下なら null。受信量が合計を超えても 100 で止める。 */
export function progressPercent(downloaded: number, total: number | null): number | null {
  if (total === null || !(total > 0)) {
    return null;
  }
  const ratio = Math.min(Math.max(downloaded, 0) / total, 1);
  return Math.floor(ratio * 100);
}

/** ダウンロード中・インストーラー起動待ちは操作を受け付けない。 */
export function isBusy(state: UpdateState): boolean {
  return state.kind === "downloading" || state.kind === "installing";
}

/** Esc・背景クリックで閉じてよいか。ダウンロード中に閉じて状況を見失わせない。 */
export function canDismiss(state: UpdateState): boolean {
  return !isBusy(state);
}

/** "12.3 MB" 形式。1 MB 未満は KB。 */
export function formatBytes(bytes: number): string {
  const safe = Math.max(0, bytes);
  if (safe < 1024 * 1024) {
    return `${Math.round(safe / 1024)} KB`;
  }
  return `${(safe / (1024 * 1024)).toFixed(1)} MB`;
}

/** invoke の拒否値(文字列)も Error も、利用者に見せる一行にする。 */
export function errorReason(e: unknown): string {
  if (typeof e === "string") {
    return e;
  }
  return e instanceof Error ? e.message : String(e);
}
