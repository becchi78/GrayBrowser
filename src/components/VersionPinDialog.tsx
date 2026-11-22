import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import {
  backupBeforeInstall,
  listInstallableReleases,
  prepareVersionInstall,
  type InstallableRelease,
  type PreparedInstall,
} from "../api";
import { errorText } from "../lib/errorText";
import { canDismiss, updateReducer, type UpdateState } from "../lib/updateState";
import { DownloadProgressBody } from "./DownloadProgressBody";
import { ModalShell } from "./ModalShell";

interface Props {
  open: boolean;
  onClose: () => void;
}

type Step =
  | { kind: "loading" }
  | { kind: "loadFailed"; reason: string }
  | { kind: "list"; releases: InstallableRelease[] }
  | { kind: "preparing"; version: string; releases: InstallableRelease[] }
  | { kind: "prepareFailed"; reason: string; releases: InstallableRelease[] }
  | { kind: "confirm"; prepared: PreparedInstall }
  | { kind: "backingUp"; prepared: PreparedInstall }
  | { kind: "backupFailed"; prepared: PreparedInstall; reason: string }
  | { kind: "install"; prepared: PreparedInstall };

// 開くたびに内側を作り直し、前回の一覧や選択を残さない。
export function VersionPinDialog({ open, onClose }: Props) {
  return open ? <VersionPinContent onClose={onClose} /> : null;
}

function VersionPinContent({ onClose }: { onClose: () => void }) {
  const [step, setStep] = useState<Step>({ kind: "loading" });
  const [currentVersion, setCurrentVersion] = useState("");
  // 「データベースが開けなくなる」警告を理解した、という明示の確認。
  const [acknowledged, setAcknowledged] = useState(false);
  const aliveRef = useRef(true);
  // 登録済みで、まだ使っていない Update(Rust 側のリソース)。使い終えたら解放する。
  const heldRef = useRef<PreparedInstall | null>(null);
  const primaryRef = useRef<HTMLButtonElement>(null);

  const release = useCallback(() => {
    const held = heldRef.current;
    heldRef.current = null;
    held?.update.close().catch(() => {});
  }, []);

  const fetchList = useCallback(() => {
    listInstallableReleases()
      .then((releases) => {
        if (aliveRef.current) setStep({ kind: "list", releases });
      })
      .catch((e) => {
        console.error(e);
        if (aliveRef.current) setStep({ kind: "loadFailed", reason: errorText(e) });
      });
  }, []);

  // 初回は state の初期値が loading なので、取得だけを始める。やり直しでは表示も戻す。
  const loadList = useCallback(() => {
    setStep({ kind: "loading" });
    fetchList();
  }, [fetchList]);

  useEffect(() => {
    aliveRef.current = true;
    getVersion()
      .then((v) => {
        if (aliveRef.current) setCurrentVersion(v);
      })
      .catch(() => {});
    fetchList();
    return () => {
      aliveRef.current = false;
      release();
    };
  }, [fetchList, release]);

  useEffect(() => {
    primaryRef.current?.focus();
  }, [step.kind]);

  function select(version: string, releases: InstallableRelease[]) {
    setStep({ kind: "preparing", version, releases });
    prepareVersionInstall(version)
      .then((prepared) => {
        if (!aliveRef.current) {
          prepared.update.close().catch(() => {});
          return;
        }
        heldRef.current = prepared;
        setAcknowledged(false);
        setStep({ kind: "confirm", prepared });
      })
      .catch((e) => {
        console.error(e);
        if (aliveRef.current) setStep({ kind: "prepareFailed", reason: errorText(e), releases });
      });
  }

  function backToList() {
    release();
    loadList();
  }

  // バックアップが要るのは、ダウングレードと、その版が今のDBを開けない場合。成功した
  // ときだけインストールへ進み、失敗したら進まない。
  function proceed(prepared: PreparedInstall) {
    if (!prepared.backupRequired) {
      startInstall(prepared);
      return;
    }
    setStep({ kind: "backingUp", prepared });
    backupBeforeInstall()
      .then(() => {
        if (aliveRef.current) startInstall(prepared);
      })
      .catch((e) => {
        console.error(e);
        if (aliveRef.current) setStep({ kind: "backupFailed", prepared, reason: errorText(e) });
      });
  }

  function startInstall(prepared: PreparedInstall) {
    // 以後の Update の扱いは InstallRun に移る。
    heldRef.current = null;
    setStep({ kind: "install", prepared });
  }

  if (step.kind === "install") {
    return <InstallRun prepared={step.prepared} onClose={onClose} />;
  }

  const busy = step.kind === "preparing" || step.kind === "backingUp";
  let body;
  let footer;
  switch (step.kind) {
    case "loading":
      body = <p data-testid="version-pin-loading">リリースの一覧を取得しています...</p>;
      footer = <CloseButton onClose={onClose} />;
      break;
    case "loadFailed":
      body = (
        <>
          <p>リリースの一覧を取得できませんでした。</p>
          <p className="dialog-error dialog-error--compact" role="alert" data-testid="version-pin-error">
            {step.reason}
          </p>
        </>
      );
      footer = (
        <div className="settings-dialog-actions">
          <CloseButton onClose={onClose} />
          <button type="button" ref={primaryRef} data-testid="version-pin-retry-btn" onClick={loadList}>
            再試行
          </button>
        </div>
      );
      break;
    case "list":
    case "preparing":
    case "prepareFailed": {
      const { releases } = step;
      const preparing = step.kind === "preparing";
      body = (
        <>
          <p className="dialog-hint">
            インストールするバージョンを選んでください。現在より古いバージョンを選ぶと、データベースが開けなくなることがあります。
          </p>
          {releases.length === 0 ? (
            <p data-testid="version-pin-empty">インストールできるリリースがありません。</p>
          ) : (
            <ul className="version-pin-list" data-testid="version-pin-list">
              {releases.map((r) => (
                <li key={r.version}>
                  <button
                    type="button"
                    className="version-pin-item"
                    data-testid={`version-pin-item-${r.version}`}
                    disabled={preparing}
                    onClick={() => select(r.version, releases)}
                  >
                    <span>{r.version}</span>
                    {r.version === currentVersion && (
                      <span className="version-pin-current" data-testid="version-pin-current">
                        現在
                      </span>
                    )}
                    {r.publishedAt !== null && (
                      <span className="version-pin-date">{r.publishedAt.slice(0, 10)}</span>
                    )}
                  </button>
                </li>
              ))}
            </ul>
          )}
          {preparing && <p data-testid="version-pin-preparing">配布情報を取得しています...</p>}
          {step.kind === "prepareFailed" && (
            <p className="dialog-error dialog-error--compact" role="alert" data-testid="version-pin-error">
              {step.reason}
            </p>
          )}
        </>
      );
      footer = preparing ? null : <CloseButton onClose={onClose} />;
      break;
    }
    case "confirm":
    case "backingUp":
    case "backupFailed": {
      const { prepared } = step;
      const tooNew = prepared.schemaWarning === "tooNew";
      body = (
        <>
          <p data-testid="version-pin-target">
            バージョン {prepared.update.version} をインストールします(現在のバージョン{" "}
            {prepared.update.currentVersion})。
          </p>
          {prepared.isDowngrade && (
            <p role="note" data-testid="version-pin-downgrade-notice">
              現在より古いバージョンです。インストールの前に、データベースを自動でバックアップします。データベースのスキーマが新しいと、古いバージョンは起動できません。
            </p>
          )}
          {tooNew && (
            <p className="dialog-error dialog-error--compact" role="alert" data-testid="version-pin-too-new">
              このバージョンは、現在のデータベース(スキーマ版 {prepared.currentSchemaVersion})を開けません(このバージョンのスキーマ版は{" "}
              {prepared.targetSchemaVersion})。インストールすると、アプリを起動できなくなります。
            </p>
          )}
          {prepared.schemaWarning === "unknown" && (
            <p
              className="update-dialog-preview-notice"
              role="note"
              data-testid="version-pin-unknown-schema"
            >
              このバージョンは配布情報にスキーマ版がなく、現在のデータベースを開けるかどうか判定できません。
            </p>
          )}
          {tooNew && (
            <label className="settings-dialog-check">
              <input
                type="checkbox"
                data-testid="version-pin-acknowledge"
                checked={acknowledged}
                disabled={step.kind !== "confirm"}
                onChange={(e) => setAcknowledged(e.target.checked)}
              />
              起動できなくなることを理解したうえで、インストールする
            </label>
          )}
          {step.kind === "backingUp" && <p data-testid="version-pin-backing-up">バックアップを作成しています...</p>}
          {step.kind === "backupFailed" && (
            <p className="dialog-error dialog-error--compact" role="alert" data-testid="version-pin-error">
              {step.reason}
            </p>
          )}
        </>
      );
      footer =
        step.kind === "backingUp" ? null : (
          <div className="settings-dialog-actions">
            <button type="button" data-testid="version-pin-back-btn" onClick={backToList}>
              戻る
            </button>
            <button
              type="button"
              ref={primaryRef}
              data-testid="version-pin-install-btn"
              disabled={tooNew && !acknowledged}
              onClick={() => proceed(prepared)}
            >
              {step.kind === "backupFailed" ? "再試行" : "インストール"}
            </button>
          </div>
        );
      break;
    }
  }

  return (
    <ModalShell
      label="バージョンを指定してインストール"
      className="update-dialog"
      testId="version-pin-dialog"
      onDismiss={onClose}
      dismissible={!busy}
    >
      <h2>バージョンを指定してインストール</h2>
      {body}
      {footer !== null && <div className="dialog-footer update-dialog-footer">{footer}</div>}
    </ModalShell>
  );
}

function CloseButton({ onClose }: { onClose: () => void }) {
  return (
    <button type="button" data-testid="version-pin-close-btn" onClick={onClose}>
      閉じる
    </button>
  );
}

// 確認の後、そのままダウンロードして適用する。ダウンロードの進捗・失敗と再試行は、
// 更新の確認ダイアログと同じ状態遷移(updateReducer)と表示を使う。
function InstallRun({ prepared, onClose }: { prepared: PreparedInstall; onClose: () => void }) {
  const [state, dispatch] = useReducer(
    updateReducer,
    prepared,
    (p): UpdateState => ({
      kind: "available",
      update: {
        version: p.update.version,
        currentVersion: p.update.currentVersion,
        notes: null,
        channel: p.update.channel,
      },
    }),
  );
  const primaryRef = useRef<HTMLButtonElement>(null);
  // 再描画を待たず、同期的に二重の適用を止める。
  const applyingRef = useRef(false);
  const startedRef = useRef(false);

  const apply = useCallback(() => {
    if (applyingRef.current) return;
    applyingRef.current = true;
    dispatch({ type: "downloadStarted" });
    prepared.update
      .downloadAndInstall((event) => dispatch({ type: "downloadEvent", event }))
      .catch((e) => {
        console.error(e);
        dispatch({ type: "downloadFailed", reason: errorText(e) });
      })
      .finally(() => {
        applyingRef.current = false;
      });
  }, [prepared]);

  useEffect(() => {
    if (startedRef.current) return;
    startedRef.current = true;
    apply();
  }, [apply]);

  useEffect(() => {
    primaryRef.current?.focus();
  }, [state.kind]);

  const failed = state.kind === "error";
  return (
    <ModalShell
      label="バージョンを指定してインストール"
      className="update-dialog"
      testId="version-pin-dialog"
      onDismiss={onClose}
      dismissible={canDismiss(state)}
    >
      <h2>バージョンを指定してインストール</h2>
      {(state.kind === "downloading" || state.kind === "installing") && <DownloadProgressBody state={state} />}
      {state.kind === "available" && <p>準備しています...</p>}
      {failed && (
        <>
          <p>インストールに失敗しました。</p>
          <p className="dialog-error dialog-error--compact" role="alert" data-testid="version-pin-error">
            {state.reason}
          </p>
          <div className="dialog-footer update-dialog-footer">
            <div className="settings-dialog-actions">
              <CloseButton onClose={onClose} />
              <button type="button" ref={primaryRef} data-testid="version-pin-retry-btn" onClick={apply}>
                再試行
              </button>
            </div>
          </div>
        </>
      )}
    </ModalShell>
  );
}
