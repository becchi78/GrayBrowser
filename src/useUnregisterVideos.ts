import { useState } from "react";
import { unregisterVideos } from "./api";

// 一覧から選んだ動画を登録から削除する流れ(対象を保留して確認を出し、確定で
// 実行する)。pending が null のとき確認は閉じている。失敗の理由は error に残し、
// 対象を選び直す・キャンセルする・確定で実行し直すと消す。
export function useUnregisterVideos() {
  const [pending, setPending] = useState<string[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function request(ids: string[]) {
    setError(null);
    setPending(ids);
  }

  function cancel() {
    setPending(null);
    setError(null);
  }

  async function run() {
    if (pending === null || busy) return;
    setBusy(true);
    setError(null);
    try {
      await unregisterVideos(pending);
      setPending(null);
    } catch (e) {
      console.error(e);
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return { pending, busy, error, request, cancel, run };
}
