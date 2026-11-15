import { useEffect, useState } from "react";
import {
  deleteDuplicateVideo,
  deleteDuplicateVideoAndFile,
  listDuplicateGroups,
  refreshDuplicateGroups,
} from "./api";
import { onDedupUpdated } from "./events";
import { withoutKey } from "./lib/withoutKey";
import type { DuplicateGroup, DuplicateGroupMember } from "./types";
import { errorText } from "./lib/errorText";

interface PendingAction {
  kind: "unregister" | "file";
  member: DuplicateGroupMember;
}

// 重複パネルの状態と操作(一覧の取得と dedup:updated の購読、再チェック、
// 確認を挟んだ削除、件数の通知、閉じたときの保留の破棄)。パネルを閉じていても
// 呼び出し側はマウントを保つので、取得・購読・件数の通知は閉じていても動く。
export function useDuplicateGroups(
  refreshKey: number,
  open: boolean,
  onCountChange?: (count: number) => void,
) {
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
      setError(errorText(e));
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
        setError(errorText(e));
      } else {
        setFileDeleteErrors((prev) => ({ ...prev, [member.video_id]: errorText(e) }));
      }
    } finally {
      setBusy(false);
      setPending(null);
    }
  }

  // 次の操作を始めた時点で、前回の失敗表示を消す。行のエラーは同じ行の
  // どちらの操作でも消し、他の行のエラーは残す。
  function clearErrorsForNewAction(member: DuplicateGroupMember) {
    setError(null);
    setFileDeleteErrors((prev) => withoutKey(prev, member.video_id));
  }

  function requestFileDelete(member: DuplicateGroupMember) {
    clearErrorsForNewAction(member);
    setPending({ kind: "file", member });
  }

  function requestUnregister(member: DuplicateGroupMember) {
    clearErrorsForNewAction(member);
    setPending({ kind: "unregister", member });
  }

  function cancelPending() {
    setPending(null);
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

  return {
    groups,
    checking,
    error,
    pending,
    busy,
    fileDeleteErrors,
    handleRefresh,
    runPendingAction,
    requestFileDelete,
    requestUnregister,
    cancelPending,
  };
}
