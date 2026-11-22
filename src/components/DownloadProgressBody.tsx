import { formatBytes, progressPercent, type UpdateState } from "../lib/updateState";

// ダウンロード中と、署名検証・インストーラー起動待ちの表示。更新の確認ダイアログと、
// バージョン指定のインストールが同じ見た目・同じ文言で進捗を出すために共有する。
export function DownloadProgressBody({
  state,
}: {
  state: Extract<UpdateState, { kind: "downloading" | "installing" }>;
}) {
  if (state.kind === "installing") {
    return (
      <p data-testid="update-dialog-installing">
        署名を検証し、インストーラーを起動しています。アプリは自動的に終了し、更新後に再起動します。
      </p>
    );
  }
  const percent = progressPercent(state.downloaded, state.total);
  return (
    <>
      <p>バージョン {state.update.version} をダウンロードしています...</p>
      <progress
        className="update-dialog-progress"
        data-testid="update-dialog-progress"
        max={100}
        value={percent ?? undefined}
      />
      <p className="dialog-hint" data-testid="update-dialog-progress-text">
        {percent === null
          ? formatBytes(state.downloaded)
          : `${percent}%(${formatBytes(state.downloaded)}${state.total !== null ? ` / ${formatBytes(state.total)}` : ""})`}
      </p>
    </>
  );
}
