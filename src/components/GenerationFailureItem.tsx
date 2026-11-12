import { splitDirAndFileName } from "../lib/paths";

// 生成失敗1件の行。サムネイル失敗とメタデータ失敗は同じ形で、バッジの文言と
// 失敗回数の出どころだけが違う。
export function GenerationFailureItem({
  badge,
  fileName,
  filePath,
  attempts,
  onRetry,
  disabled,
}: {
  badge: string;
  fileName: string;
  filePath: string;
  attempts: number;
  onRetry: () => void;
  disabled: boolean;
}) {
  return (
    <li className="generation-failure-item">
      <span className="generation-failure-badge">{badge}</span>
      <div className="generation-failure-info">
        <span className="generation-failure-name">{fileName}</span>
        <FilePathRow filePath={filePath} />
        <span className="generation-failure-attempts">失敗回数: {attempts}</span>
      </div>
      <button type="button" onClick={onRetry} disabled={disabled}>
        再試行
      </button>
    </li>
  );
}

// The path row of GenerationFailureItem above. See App.css's
// `.file-path-row` comment for why the path is split at all.
function FilePathRow({ filePath }: { filePath: string }) {
  const { dir, name } = splitDirAndFileName(filePath);
  return (
    <span className="file-path-row" title={filePath}>
      <span className="file-path-dir">{dir}</span>
      <span className="file-path-file">{name}</span>
    </span>
  );
}
