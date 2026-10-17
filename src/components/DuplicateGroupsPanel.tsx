import { useEffect, useState } from "react";
import {
  deleteDuplicateVideo,
  deleteDuplicateVideoAndFile,
  getThumbnails,
  listDuplicateGroups,
  playVideo,
  refreshDuplicateGroups,
} from "../api";
import { ConfirmDialog } from "./ConfirmDialog";
import { onDedupUpdated } from "../events";
import { fileDeleteDisabledReason } from "../lib/duplicateDelete";
import { computeDuplicateDiff, type DuplicateDiff } from "../lib/duplicateDiff";
import { formatDuration, formatFileSize, formatResolution } from "../lib/format";
import type { DuplicateGroup, DuplicateGroupKind, DuplicateGroupMember } from "../types";

const KIND_LABELS: Record<DuplicateGroupKind, string> = {
  quick_hash_confirmed: "内容一致（確定）",
  path_collision_confirmed: "パス衝突・内容確認済み",
  path_collision_unconfirmed: "パス衝突・内容未確認",
};

interface PendingAction {
  kind: "unregister" | "file";
  member: DuplicateGroupMember;
}

interface Props {
  refreshKey: number;
  // Visibility is driven by the StatusBar badge, not this component's own
  // toggle button. The component stays mounted regardless of `open` so
  // the fetch effect and the onDedupUpdated subscription below keep the
  // badge's count fresh in the background even while the panel is closed.
  open: boolean;
  // Reports the current total member count up to StatusBar for the badge
  // label.
  onCountChange?: (count: number) => void;
}

export function DuplicateGroupsPanel({ refreshKey, open, onCountChange }: Props) {
  const [groups, setGroups] = useState<DuplicateGroup[]>([]);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingAction | null>(null);
  const [busy, setBusy] = useState(false);
  const [fileDeleteErrors, setFileDeleteErrors] = useState<Record<string, string>>({});

  useEffect(() => {
    listDuplicateGroups().then(setGroups).catch(console.error);
  }, [refreshKey]);

  useEffect(() => {
    const unlisten = onDedupUpdated((updated) => {
      setGroups(updated);
      setChecking(false);
    });
    return () => {
      unlisten();
    };
  }, []);

  async function handleRefresh() {
    setChecking(true);
    setError(null);
    try {
      await refreshDuplicateGroups();
      // Completion (and the resulting group list) arrives asynchronously via
      // dedup:updated, which also clears `checking`.
    } catch (e) {
      console.error(e);
      setError(String(e));
      setChecking(false);
    }
  }

  // 確認が済むまでバックエンドを呼ばないため、操作はいったん state に
  // 保留し、確認モーダルの確定ボタンでだけ実行する。window.confirm は
  // 使えない（ConfirmDialog の冒頭コメント参照）。
  async function runPendingAction() {
    if (!pending || busy) {
      return;
    }
    const { kind, member } = pending;
    setBusy(true);
    try {
      if (kind === "unregister") {
        await deleteDuplicateVideo(member.video_id);
      } else {
        await deleteDuplicateVideoAndFile(member.video_id);
      }
      removeMemberFromGroups(member.video_id);
    } catch (e) {
      console.error(e);
      if (kind === "unregister") {
        setError(String(e));
      } else {
        setFileDeleteErrors((prev) => ({ ...prev, [member.video_id]: String(e) }));
      }
    } finally {
      setBusy(false);
      setPending(null);
    }
  }

  function requestFileDelete(member: DuplicateGroupMember) {
    setFileDeleteErrors((prev) => {
      const next = { ...prev };
      delete next[member.video_id];
      return next;
    });
    setPending({ kind: "file", member });
  }

  function removeMemberFromGroups(videoId: string) {
    setGroups((prev) =>
      prev
        .map((g) => ({
          ...g,
          members: g.members.filter((m) => m.video_id !== videoId),
        }))
        .filter((g) => g.members.length > 0),
    );
  }

  const totalMembers = groups.reduce((sum, g) => sum + g.members.length, 0);

  useEffect(() => {
    onCountChange?.(totalMembers);
    // Fires whenever `groups` (and therefore totalMembers) changes,
    // regardless of which of the three update paths above (fetch/event/
    // local delete) caused it. `onCountChange` is always a stable
    // useState setter passed by StatusBar.
  }, [totalMembers, onCountChange]);

  // パネルを閉じたまま保留中の確認だけが残らないようにする。
  useEffect(() => {
    if (!open && !busy) {
      setPending(null);
    }
  }, [open, busy]);

  if (!open) {
    return null;
  }

  return (
    <section className="duplicate-groups-panel status-panel" data-testid="status-panel">
      <div className="duplicate-groups-panel-row">
        <button type="button" onClick={handleRefresh} disabled={checking}>
          {checking ? "確認中..." : "重複を再チェック"}
        </button>
      </div>
      {error && <p className="duplicate-groups-error">{error}</p>}
      {groups.length === 0 ? (
        <p>重複候補は見つかっていません。</p>
      ) : (
        <ul className="duplicate-groups-list">
          {groups.map((group, index) => (
            <DuplicateGroupItem
              key={index}
              group={group}
              busy={busy}
              fileDeleteErrors={fileDeleteErrors}
              onDelete={(member) => setPending({ kind: "unregister", member })}
              onDeleteFile={requestFileDelete}
            />
          ))}
        </ul>
      )}
      <ConfirmDialog
        open={pending !== null}
        title={pending?.kind === "file" ? "ファイルも削除" : "登録解除"}
        confirmLabel={pending?.kind === "file" ? "ゴミ箱に移動して削除" : "登録を解除する"}
        danger={pending?.kind === "file"}
        busy={busy}
        onConfirm={runPendingAction}
        onCancel={() => setPending(null)}
      >
        {pending?.kind === "file" && (
          <>
            <p>この動画ファイルをゴミ箱に移動し、カタログからも削除します。</p>
            <p>
              <strong>ファイル名:</strong> {pending.member.file_name}
            </p>
            <p className="confirm-dialog-path">{pending.member.file_path}</p>
            <p>ゴミ箱に移動します（ゴミ箱から元に戻せます）。</p>
          </>
        )}
        {pending?.kind === "unregister" && (
          <>
            <p>このカタログ登録を解除します。</p>
            <p className="confirm-dialog-path">{pending.member.file_path}</p>
            <p>
              元の動画ファイルは削除されません。GrayBrowser上のタグ・評価などカタログの情報は失われます。
            </p>
          </>
        )}
      </ConfirmDialog>
    </section>
  );
}

function DuplicateGroupItem({
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

function diffClass(differs: boolean): string | undefined {
  return differs ? "duplicate-group-member-diff" : undefined;
}

function DuplicateGroupMemberRow({
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
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [playError, setPlayError] = useState<string | null>(null);
  const isOffline = member.status === "offline";

  async function handlePlay() {
    if (isOffline) return;
    setPlayError(null);
    try {
      await playVideo(member.file_path);
    } catch (e) {
      console.error(e);
      setPlayError(`再生に失敗しました: ${String(e)}`);
    }
  }

  useEffect(() => {
    let cancelled = false;
    getThumbnails(member.video_id)
      .then((urls) => {
        if (cancelled) return;
        // 6枚配列の先頭1枚だけを従来どおりの単一サムネイルとして表示する
        // （複数枚表示には対応していない）。
        setDataUrl(urls?.[0] ?? null);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [member.video_id]);

  return (
    <li
      className={
        isOffline
          ? "duplicate-group-member duplicate-group-member--offline"
          : "duplicate-group-member"
      }
    >
      <div
        className={
          isOffline
            ? "duplicate-group-member-thumb"
            : "duplicate-group-member-thumb duplicate-group-member-thumb--playable"
        }
        onClick={isOffline ? undefined : handlePlay}
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
        className="duplicate-group-member-play"
        onClick={handlePlay}
        disabled={isOffline}
      >
        再生
      </button>
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
        ファイルも削除
      </button>
    </li>
  );
}
