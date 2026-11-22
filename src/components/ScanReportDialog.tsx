import { useEffect, useRef, useState } from "react";
import { getScanReport } from "../api";
import { errorText } from "../lib/errorText";
import { ModalShell } from "./ModalShell";

interface Props {
  open: boolean;
  onClose: () => void;
}

type CopyState = "idle" | "copied" | "failed";

// 開くたびに内側を作り直し、前回の内容を state に残さない。
export function ScanReportDialog({ open, onClose }: Props) {
  return open ? <ScanReportContent onClose={onClose} /> : null;
}

function ScanReportContent({ onClose }: { onClose: () => void }) {
  // undefined: 取得中 / null: まだスキャンが実行されていない
  const [report, setReport] = useState<string | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [copyState, setCopyState] = useState<CopyState>("idle");
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let cancelled = false;
    getScanReport()
      .then((text) => {
        if (!cancelled) setReport(text);
      })
      .catch((e) => {
        console.error(e);
        if (!cancelled) setError(errorText(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    closeRef.current?.focus();
  }, []);

  function copy() {
    if (typeof report !== "string") return;
    navigator.clipboard
      .writeText(report)
      .then(() => setCopyState("copied"))
      .catch((e) => {
        console.error(e);
        setCopyState("failed");
      });
  }

  return (
    <ModalShell
      label="スキャン診断"
      className="scan-report-dialog"
      testId="scan-report-dialog"
      onDismiss={onClose}
    >
      <h2>スキャン診断</h2>
      <p className="scan-report-dialog-note">
        直近の手動スキャンの、段階別の所要時間と失敗したファイルです。アプリを終了すると消えます。
        ユーザー名を含むパスは置き換えてありますが、貼り付ける前に内容を確認してください。
      </p>
      {error !== null ? (
        <p className="scan-report-dialog-error" role="alert" data-testid="scan-report-error">
          {error}
        </p>
      ) : report === null ? (
        <p data-testid="scan-report-empty">
          まだスキャンが実行されていません。フォルダをスキャンすると、ここに結果が出ます。
        </p>
      ) : (
        <textarea
          className="scan-report-dialog-text"
          readOnly
          value={report ?? "取得中..."}
          data-testid="scan-report-text"
          aria-label="スキャン診断"
        />
      )}
      <div className="dialog-footer">
        <span className="scan-report-dialog-status" role="status" data-testid="scan-report-status">
          {copyState === "copied" && "コピーしました"}
          {copyState === "failed" && "コピーできませんでした"}
        </span>
        <span className="scan-report-dialog-buttons">
          <button
            type="button"
            data-testid="scan-report-copy-btn"
            disabled={typeof report !== "string"}
            onClick={copy}
          >
            コピー
          </button>
          <button type="button" ref={closeRef} data-testid="scan-report-close-btn" onClick={onClose}>
            閉じる
          </button>
        </span>
      </div>
    </ModalShell>
  );
}
