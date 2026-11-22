import { useEffect, useRef, useState } from "react";
import {
  getDbStats,
  runIntegrityCheck,
  runVacuum,
  type DbStats,
  type IntegrityResult,
  type VacuumResult,
} from "../api";
import { errorText } from "../lib/errorText";
import { ConfirmDialog } from "./ConfirmDialog";
import { ModalShell } from "./ModalShell";

interface Props {
  open: boolean;
  onClose: () => void;
}

type VacuumState =
  | { kind: "idle" }
  | { kind: "confirming" }
  | { kind: "running" }
  | { kind: "done"; result: VacuumResult }
  | { kind: "error"; message: string };

// 開くたびに内側を作り直し、前回の結果を state に残さない。
export function DbToolsDialog({ open, onClose }: Props) {
  return open ? <DbToolsContent onClose={onClose} /> : null;
}

function DbToolsContent({ onClose }: { onClose: () => void }) {
  const [stats, setStats] = useState<DbStats | null>(null);
  const [statsError, setStatsError] = useState<string | null>(null);
  const [integrity, setIntegrity] = useState<IntegrityResult | null>(null);
  const [integrityError, setIntegrityError] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [vacuum, setVacuum] = useState<VacuumState>({ kind: "idle" });
  const closeRef = useRef<HTMLButtonElement>(null);
  const aliveRef = useRef(true);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  function loadStats() {
    getDbStats()
      .then((s) => {
        if (aliveRef.current) {
          setStats(s);
          setStatsError(null);
        }
      })
      .catch((e) => {
        console.error(e);
        if (aliveRef.current) setStatsError(errorText(e));
      });
  }

  useEffect(loadStats, []);

  useEffect(() => {
    closeRef.current?.focus();
  }, []);

  const vacuumRunning = vacuum.kind === "running";
  const busy = checking || vacuumRunning;

  function check() {
    setChecking(true);
    setIntegrity(null);
    setIntegrityError(null);
    runIntegrityCheck()
      .then((r) => {
        if (aliveRef.current) setIntegrity(r);
      })
      .catch((e) => {
        console.error(e);
        if (aliveRef.current) setIntegrityError(errorText(e));
      })
      .finally(() => {
        if (aliveRef.current) setChecking(false);
      });
  }

  function execVacuum() {
    setVacuum({ kind: "running" });
    runVacuum()
      .then((result) => {
        if (!aliveRef.current) return;
        setVacuum({ kind: "done", result });
        loadStats();
      })
      .catch((e) => {
        console.error(e);
        if (aliveRef.current)
          setVacuum({ kind: "error", message: errorText(e) });
      });
  }

  return (
    <>
      <ModalShell
        label="データベースツール"
        className="db-tools-dialog"
        testId="db-tools-dialog"
        onDismiss={onClose}
        dismissible={!vacuumRunning}
      >
        <h2>データベースツール</h2>

        <section className="db-tools-section" data-testid="db-tools-stats">
          <h3>サイズ</h3>
          {statsError !== null ? (
            <p className="db-tools-error" role="alert">
              {statsError}
            </p>
          ) : stats === null ? (
            <p>取得中...</p>
          ) : (
            <dl className="db-tools-stats-list">
              <dt>ファイルサイズ</dt>
              <dd data-testid="db-tools-file-size">{stats.file_size_text}</dd>
              <dt>ページ数</dt>
              <dd>{stats.page_count.toLocaleString()}</dd>
              <dt>空きページ数</dt>
              <dd>
                {stats.freelist_count.toLocaleString()}(最適化で約{" "}
                {stats.reclaimable_text} を返せる見込み)
              </dd>
            </dl>
          )}
        </section>

        <section className="db-tools-section">
          <h3>整合性チェック</h3>
          <p className="db-tools-note">
            データベースが壊れていないかを調べます。何も変更しません。
          </p>
          <div>
            <button
              type="button"
              data-testid="db-tools-check-btn"
              disabled={busy}
              onClick={check}
            >
              {checking ? "調べています..." : "整合性チェックを実行"}
            </button>
          </div>
          {integrityError !== null && (
            <p
              className="db-tools-error"
              role="alert"
              data-testid="db-tools-integrity-error"
            >
              {integrityError}
            </p>
          )}
          {integrity !== null &&
            (integrity.ok ? (
              <p role="status" data-testid="db-tools-integrity-ok">
                問題は見つかりませんでした。
              </p>
            ) : (
              <div role="alert" data-testid="db-tools-integrity-ng">
                <p className="db-tools-error">問題が見つかりました。</p>
                <ul className="db-tools-integrity-list">
                  {integrity.messages.map((m, i) => (
                    <li key={i}>{m}</li>
                  ))}
                </ul>
              </div>
            ))}
        </section>

        <section className="db-tools-section">
          <h3>最適化(VACUUM)</h3>
          <p className="db-tools-note">
            空き領域を回収してデータベースのファイルを小さくします。実行前に自動でバックアップを取ります。
          </p>
          <div>
            <button
              type="button"
              data-testid="db-tools-vacuum-btn"
              disabled={busy}
              onClick={() => setVacuum({ kind: "confirming" })}
            >
              最適化を実行...
            </button>
          </div>
          {vacuum.kind === "done" && (
            <p role="status" data-testid="db-tools-vacuum-done">
              最適化が完了しました({vacuum.result.size_before_text} →{" "}
              {vacuum.result.size_after_text}
              )。実行前のバックアップ: {vacuum.result.backup_file}
            </p>
          )}
          {vacuum.kind === "error" && (
            <p
              className="db-tools-error"
              role="alert"
              data-testid="db-tools-vacuum-error"
            >
              {vacuum.message}
            </p>
          )}
        </section>

        <div className="dialog-footer">
          <button
            type="button"
            ref={closeRef}
            data-testid="db-tools-close-btn"
            disabled={vacuumRunning}
            onClick={onClose}
          >
            閉じる
          </button>
        </div>
      </ModalShell>
      <ConfirmDialog
        open={vacuum.kind === "confirming" || vacuum.kind === "running"}
        title="データベースを最適化しますか"
        confirmLabel="最適化する"
        danger
        busy={vacuumRunning}
        onConfirm={execVacuum}
        onCancel={() => setVacuum({ kind: "idle" })}
      >
        <p>
          実行前に、データベースのバックアップを自動で取ります。バックアップに失敗したときは、最適化を行いません。
        </p>
        <p>
          最適化そのものは元に戻せません。完了までに時間がかかることがあり、その間は、ほかの操作が待たされることがあります。
        </p>
      </ConfirmDialog>
    </>
  );
}
