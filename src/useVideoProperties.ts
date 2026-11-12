import { useEffect, useState } from "react";
import { getVideoProperties } from "./api";
import { needsPropertiesRefresh } from "./lib/videoProperties";
import type { VideoPropertiesDto } from "./types";

// 一覧の行に出す動画のプロパティ(解像度・コーデックなど)。まだ取得が
// 終わっていないあいだは stillProbing が true で、一覧の更新のたびに取り直す。
export function useVideoProperties(videoId: string, catalogVersion: number) {
  const [properties, setProperties] = useState<VideoPropertiesDto | null>(null);

  // `probed_at === null` covers both "not yet probed" and "probe failed
  // permanently" -- the backend doesn't distinguish these today (known
  // limitation). Showing "取得中..." is the common case; this is a
  // deliberate simplification, not a claim that it's always still in
  // progress.
  const stillProbing = needsPropertiesRefresh(properties);

  // バックグラウンドのメタデータ取得は1件終わるごとに一覧の更新通知を出すが、
  // 行は再マウントされないため、マウント時の一度きりの取得では、その時点で
  // 未取得だった行が「取得中...」のまま固まる。そこで一覧の更新（catalogVersion）
  // を契機に、未取得の行だけ取り直す。stillProbing（boolean）を依存に使うのは、
  // 未取得のまま応答が返っても値が変わらず、effectがループしないようにするため。
  // ffprobeに永続的に失敗した行は更新のたびに取り直すが、対象は仮想スクロールで
  // 描画中の数行に限られ負荷は小さい。ポーリングはせずイベント駆動にそろえる。
  useEffect(() => {
    if (!stillProbing) return;
    let cancelled = false;
    getVideoProperties(videoId)
      .then((props) => {
        if (!cancelled) setProperties(props);
      })
      .catch(console.error);
    return () => {
      cancelled = true;
    };
  }, [videoId, catalogVersion, stillProbing]);

  return { properties, stillProbing };
}
