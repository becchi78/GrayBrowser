import { useEffect, useRef, useState } from "react";
import { getDiagnostics } from "../api";
import { errorText } from "../lib/errorText";
import { ModalShell } from "./ModalShell";

interface Props {
  open: boolean;
  onClose: () => void;
}

type CopyState = "idle" | "copied" | "failed";

// 開くたびに内側を作り直し、前回の内容を state に残さない。
export function DiagnosticsDialog({ open, onClose }: Props) {
  return open ? <DiagnosticsContent onClose={onClose} /> : null;
}

function DiagnosticsContent({ onClose }: { onClose: () => void }) {
  const [report, setReport] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copyState, setCopyState] = useState<CopyState>("idle");
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let cancelled = false;
    getDiagnostics()
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
    if (report === null) return;
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
      label="診断情報"
      className="diagnostics-dialog"
      testId="diagnostics-dialog"
      onDismiss={onClose}
    >
      <h2>診断情報</h2>
      <p className="diagnostics-dialog-note">
        不具合の報告に貼り付けて使えます。ユーザー名を含むパスは置き換えてありますが、
        貼り付ける前に内容を確認してください。
      </p>
      {error !== null ? (
        <p className="diagnostics-dialog-error" role="alert" data-testid="diagnostics-error">
          {error}
        </p>
      ) : (
        <textarea
          className="diagnostics-dialog-text"
          readOnly
          value={report ?? "取得中..."}
          data-testid="diagnostics-text"
          aria-label="診断情報"
        />
      )}
      <div className="dialog-footer">
        <span className="diagnostics-dialog-status" role="status" data-testid="diagnostics-status">
          {copyState === "copied" && "コピーしました"}
          {copyState === "failed" && "コピーできませんでした"}
        </span>
        <span className="diagnostics-dialog-buttons">
          <button
            type="button"
            data-testid="diagnostics-copy-btn"
            disabled={report === null}
            onClick={copy}
          >
            コピー
          </button>
          <button type="button" ref={closeRef} data-testid="diagnostics-close-btn" onClick={onClose}>
            閉じる
          </button>
        </span>
      </div>
    </ModalShell>
  );
}
