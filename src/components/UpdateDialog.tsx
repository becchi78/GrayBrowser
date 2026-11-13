import { useCallback, useEffect, useReducer, useRef } from "react";
import { getVersion } from "@tauri-apps/api/app";
import type { Update } from "@tauri-apps/plugin-updater";
import { checkForUpdate } from "../api";
import { errorText } from "../lib/errorText";
import { createLatestOnlyGuard } from "../lib/latestRequest";
import {
  canDismiss,
  formatBytes,
  INITIAL_UPDATE_STATE,
  progressPercent,
  updateReducer,
} from "../lib/updateState";
import { ModalShell } from "./ModalShell";

interface Props {
  open: boolean;
  onClose: () => void;
}

// 開くたびに内側を作り直し、前回の状態や結果を残さない。
export function UpdateDialog({ open, onClose }: Props) {
  return open ? <UpdateDialogContent onClose={onClose} /> : null;
}

function UpdateDialogContent({ onClose }: { onClose: () => void }) {
  const [state, dispatch] = useReducer(updateReducer, INITIAL_UPDATE_STATE);
  const primaryRef = useRef<HTMLButtonElement>(null);
  // 適用に使う Update(Rust 側のリソース)。state には載せない。
  const updateRef = useRef<Update | null>(null);
  const guardRef = useRef(createLatestOnlyGuard());
  // ボタンの無効化は再描画後にしか効かないため、連打で適用が二重に走らないよう同期的に止める。
  const applyingRef = useRef(false);

  const releaseUpdate = useCallback(() => {
    const held = updateRef.current;
    updateRef.current = null;
    if (held !== null) {
      held.close().catch(() => {});
    }
  }, []);

  const runCheck = useCallback(() => {
    const id = guardRef.current.begin();
    dispatch({ type: "checkStarted" });
    releaseUpdate();
    checkForUpdate()
      .then(async (update) => {
        if (!guardRef.current.isLatest(id)) {
          await update?.close().catch(() => {});
          return;
        }
        if (update === null) {
          // 現在の版は「最新です」の表示にだけ使う。取れなくても確認自体は成功している。
          const currentVersion = await getVersion().catch(() => "");
          if (guardRef.current.isLatest(id)) {
            dispatch({ type: "checkSucceeded", result: null, currentVersion });
          }
          return;
        }
        updateRef.current = update;
        dispatch({
          type: "checkSucceeded",
          currentVersion: update.currentVersion,
          result: {
            version: update.version,
            currentVersion: update.currentVersion,
            notes: update.body && update.body.trim() !== "" ? update.body : null,
          },
        });
      })
      .catch((e) => {
        console.error(e);
        if (guardRef.current.isLatest(id)) {
          dispatch({ type: "checkFailed", reason: errorText(e) });
        }
      });
  }, [releaseUpdate]);

  useEffect(() => {
    const guard = guardRef.current;
    runCheck();
    return () => {
      // 閉じた後に遅れて届く確認結果を捨てる。
      guard.begin();
      releaseUpdate();
    };
  }, [runCheck, releaseUpdate]);

  const dismissible = canDismiss(state);

  // 状態が変わって主ボタンが現れたら、キーボードですぐ押せるようにする。
  useEffect(() => {
    primaryRef.current?.focus();
  }, [state.kind]);

  function apply() {
    const update = updateRef.current;
    if (update === null || applyingRef.current) {
      return;
    }
    applyingRef.current = true;
    dispatch({ type: "downloadStarted" });
    update
      .downloadAndInstall((event) => dispatch({ type: "downloadEvent", event }))
      .catch((e) => {
        console.error(e);
        dispatch({ type: "downloadFailed", reason: errorText(e) });
      })
      .finally(() => {
        applyingRef.current = false;
      });
  }

  let body;
  let footer;
  switch (state.kind) {
    case "checking":
      body = <p data-testid="update-dialog-checking">更新を確認しています...</p>;
      footer = (
        <button type="button" data-testid="update-dialog-close-btn" onClick={onClose}>
          閉じる
        </button>
      );
      break;
    case "upToDate":
      body = (
        <p data-testid="update-dialog-up-to-date">
          最新です{state.currentVersion !== "" && `(現在のバージョン ${state.currentVersion})`}
        </p>
      );
      footer = (
        <button type="button" ref={primaryRef} data-testid="update-dialog-close-btn" onClick={onClose}>
          閉じる
        </button>
      );
      break;
    case "available":
      body = <AvailableBody update={state.update} />;
      footer = (
        <div className="settings-dialog-actions">
          <button type="button" data-testid="update-dialog-later-btn" onClick={onClose}>
            あとで
          </button>
          <button type="button" ref={primaryRef} data-testid="update-dialog-apply-btn" onClick={apply}>
            今すぐ更新
          </button>
        </div>
      );
      break;
    case "downloading": {
      const percent = progressPercent(state.downloaded, state.total);
      body = (
        <>
          <p>バージョン {state.update.version} をダウンロードしています...</p>
          <progress
            className="update-dialog-progress"
            data-testid="update-dialog-progress"
            max={100}
            value={percent ?? undefined}
          />
          <p className="dialog-hint" data-testid="update-dialog-progress-text">
            {percent === null
              ? formatBytes(state.downloaded)
              : `${percent}%(${formatBytes(state.downloaded)}${state.total !== null ? ` / ${formatBytes(state.total)}` : ""})`}
          </p>
        </>
      );
      footer = null;
      break;
    }
    case "installing":
      body = (
        <p data-testid="update-dialog-installing">
          署名を検証し、インストーラーを起動しています。アプリは自動的に終了し、更新後に再起動します。
        </p>
      );
      footer = null;
      break;
    case "error":
      body = (
        <>
          <p>
            {state.phase === "check" ? "更新を確認できませんでした。" : "更新に失敗しました。"}
          </p>
          <p className="dialog-error dialog-error--compact" role="alert" data-testid="update-dialog-error">
            {state.reason}
          </p>
        </>
      );
      footer = (
        <div className="settings-dialog-actions">
          <button type="button" data-testid="update-dialog-close-btn" onClick={onClose}>
            閉じる
          </button>
          <button
            type="button"
            ref={primaryRef}
            data-testid="update-dialog-retry-btn"
            onClick={state.phase === "check" ? runCheck : apply}
          >
            再試行
          </button>
        </div>
      );
      break;
  }

  return (
    <ModalShell
      label="更新の確認"
      className="update-dialog"
      testId="update-dialog"
      onDismiss={onClose}
      dismissible={dismissible}
    >
      <h2>更新の確認</h2>
      {body}
      {footer !== null && <div className="dialog-footer update-dialog-footer">{footer}</div>}
    </ModalShell>
  );
}

function AvailableBody({ update }: { update: { version: string; currentVersion: string; notes: string | null } }) {
  return (
    <>
      <p data-testid="update-dialog-available">
        新しいバージョン {update.version} があります(現在のバージョン {update.currentVersion})。
      </p>
      {update.notes !== null && (
        <pre className="update-dialog-notes" data-testid="update-dialog-notes">
          {update.notes}
        </pre>
      )}
      <p className="dialog-hint">
        更新するとアプリを終了してインストーラーを実行します。登録したフォルダやタグなどのデータは変わりません。
      </p>
    </>
  );
}
