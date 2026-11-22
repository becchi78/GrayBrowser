import { useEffect, useState } from "react";
import { getDataDirInfo, type DataDirInfo } from "../api";

// 開発者モードのバッジの隣に、使用中のデータフォルダを出す。
// 別のフォルダで起動しているとき(overridden)は、実データと取り違えないよう強調する。
// 取得に失敗したときは出さない(誤った場所を示すより、出さないほうが安全)。
export function DataDirIndicator() {
  const [info, setInfo] = useState<DataDirInfo | null>(null);

  useEffect(() => {
    let cancelled = false;
    // 同期で例外を投げる場合(未定義のコマンドなど)も、取得失敗として扱う。
    Promise.resolve()
      .then(() => getDataDirInfo())
      .then((value) => {
        if (!cancelled) {
          setInfo(value);
        }
      })
      .catch((e) => {
        console.error(e);
        if (!cancelled) {
          setInfo(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (!info) {
    return null;
  }
  const className = info.overridden
    ? "data-dir-indicator data-dir-indicator-overridden"
    : "data-dir-indicator";
  return (
    <span className={className} data-testid="data-dir-indicator" title={info.path}>
      {info.overridden ? "別のデータフォルダ: " : "データフォルダ: "}
      {info.path}
    </span>
  );
}
