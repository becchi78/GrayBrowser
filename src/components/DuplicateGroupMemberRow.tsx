import type { DuplicateDiff } from "../lib/duplicateDiff";
import { formatDuration, formatFileSize, formatResolution } from "../lib/format";
import type { DuplicateGroupMember } from "../types";
import { useIsPathUnreachable } from "../reachabilityContext";
import { useFirstThumbnail } from "../useFirstThumbnail";
import { useStartPlay } from "../useStartPlay";

function diffClass(differs: boolean): string | undefined {
  return differs ? "duplicate-group-member-diff" : undefined;
}

// 重複候補のメンバー1行。サムネイル・各項目・案内とエラー・操作ボタンを描く。
// 削除の状態は持たず、呼び出し側から受け取る。
export function DuplicateGroupMemberRow({
  member,
  diff,
  fileDeleteDisabledReason,
  busy,
  deleteFileError,
  onDelete,
  onDeleteFile,
}: {
  member: DuplicateGroupMember;
  diff: DuplicateDiff;
  fileDeleteDisabledReason: string | null;
  busy: boolean;
  deleteFileError: string | null;
  onDelete: () => void;
  onDeleteFile: () => void;
}) {
  // 保存先のフォルダに今届かない場合も、通常のオフラインと同じ扱いにする。
  const folderUnreachable = useIsPathUnreachable(member.file_path);
  const isOffline = member.status === "offline" || folderUnreachable;
  const dataUrl = useFirstThumbnail(member.video_id);
  const { playError, isStartingPlay, handlePlay } = useStartPlay(
    member.file_path,
    isOffline || busy,
  );

  return (
    <li
      className={
        isOffline
          ? "duplicate-group-member duplicate-group-member--offline"
          : "duplicate-group-member"
      }
    >
      <div
        className="duplicate-group-member-thumb"
        onDoubleClick={isOffline || busy ? undefined : handlePlay}
      >
        {dataUrl ? (
          <img src={dataUrl} alt={member.file_name} loading="lazy" />
        ) : (
          <div className="thumbnail-placeholder">
            {isOffline ? "オフライン" : "サムネイルなし"}
          </div>
        )}
      </div>
      <div className="duplicate-group-member-info">
        <span className="duplicate-group-member-path" title={member.file_path}>
          {member.file_path}
        </span>
        {isOffline && (
          <span className="duplicate-group-member-meta">
            <span className="thumbnail-offline-badge">オフライン</span>
          </span>
        )}
        <dl className="duplicate-group-member-props">
          <div>
            <dt>サイズ</dt>
            <dd className={diffClass(diff.file_size)}>{formatFileSize(member.file_size)}</dd>
          </div>
          <div>
            <dt>再生時間</dt>
            <dd className={diffClass(diff.duration)}>{formatDuration(member.duration)}</dd>
          </div>
          <div>
            <dt>解像度</dt>
            <dd className={diffClass(diff.resolution)}>
              {formatResolution(member.width, member.height)}
            </dd>
          </div>
          <div>
            <dt>登録日時</dt>
            <dd>{member.created_at}</dd>
          </div>
        </dl>
        {isOffline && (
          <span className="duplicate-group-member-play-note">オフラインのため再生できません</span>
        )}
        {isStartingPlay && (
          <span className="duplicate-group-member-play-note">再生を開始しています…</span>
        )}
        {playError && <span className="duplicate-group-member-play-error">{playError}</span>}
        {fileDeleteDisabledReason && (
          <span className="duplicate-group-member-play-note">{fileDeleteDisabledReason}</span>
        )}
        {deleteFileError && (
          <span className="duplicate-group-member-play-error">{deleteFileError}</span>
        )}
      </div>
      <button
        type="button"
        className="duplicate-group-member-delete"
        onClick={onDelete}
        disabled={busy}
      >
        登録解除
      </button>
      <button
        type="button"
        className="duplicate-group-member-delete-file"
        onClick={onDeleteFile}
        disabled={fileDeleteDisabledReason !== null || busy}
        title="ファイルをゴミ箱に移動し、カタログからも削除します"
      >
        ファイルを削除
      </button>
    </li>
  );
}
