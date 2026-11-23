// SkippedFilesPanel が catalog:changed を受けて一覧と件数を取り直すことを確かめる。
import { beforeEach, expect, test, vi } from "vitest";
import { act, waitFor } from "@testing-library/react";
import * as api from "../api";
import * as events from "../events";
import { settle } from "../testing/async";
import { renderWithProviders } from "../testing/render";
import type { SkippedFileDto } from "../types";
import { REFETCH_DELAY_MS } from "../useLibraryCounts";
import { SkippedFilesPanel } from "./SkippedFilesPanel";

vi.mock("../api", () => ({
  dismissSkippedFile: vi.fn(),
  listSkippedFiles: vi.fn(),
  registerMissingSkippedFile: vi.fn(),
  renameSkippedFile: vi.fn(),
}));

vi.mock("../events", () => ({
  onCatalogChanged: vi.fn(),
}));

function skipped(id: number): SkippedFileDto {
  return {
    id,
    file_path: `C:\\videos\\f${id}.mp4`,
    file_name: `f${id}.mp4`,
    reason: "machine_dependent_char",
    detected_char: "\uE000",
    file_missing: false,
  };
}

let emitCatalog: (() => void) | null = null;
let unlisten = vi.fn();

beforeEach(() => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  emitCatalog = null;
  unlisten = vi.fn();
  vi.mocked(events.onCatalogChanged).mockImplementation((cb) => {
    emitCatalog = cb;
    return unlisten;
  });
});

/** catalog:changed を n 回届け、デバウンスの待ちを進めて、取り直しが終わるまで待つ。 */
async function emitCatalogChanged(times = 1) {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
  try {
    for (let i = 0; i < times; i++) {
      await act(async () => {
        emitCatalog!();
      });
    }
    await act(async () => {
      vi.advanceTimersByTime(REFETCH_DELAY_MS);
    });
  } finally {
    vi.useRealTimers();
  }
  await settle();
}

async function renderPanel(open: boolean) {
  vi.mocked(api.listSkippedFiles).mockResolvedValue([skipped(1)]);
  const onCountChange = vi.fn();
  const view = renderWithProviders(
    <SkippedFilesPanel refreshKey={0} open={open} onCountChange={onCountChange} />,
  );
  await waitFor(() => expect(api.listSkippedFiles).toHaveBeenCalled());
  await settle();
  return { ...view, onCountChange };
}

test("閉じていても catalog:changed のあとに取り直し、件数を更新する", async () => {
  const { onCountChange } = await renderPanel(false);
  expect(onCountChange).toHaveBeenLastCalledWith(1);

  vi.mocked(api.listSkippedFiles).mockResolvedValue([skipped(1), skipped(2)]);
  await emitCatalogChanged();

  expect(api.listSkippedFiles).toHaveBeenCalledTimes(2);
  expect(onCountChange).toHaveBeenLastCalledWith(2);
});

test("待ち時間が過ぎるまでは取り直さない", async () => {
  await renderPanel(false);

  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
  try {
    await act(async () => {
      emitCatalog!();
    });
    await act(async () => {
      vi.advanceTimersByTime(REFETCH_DELAY_MS - 1);
    });
    expect(api.listSkippedFiles).toHaveBeenCalledTimes(1);
  } finally {
    vi.useRealTimers();
  }
});

test("続けて届いた通知は1回の取得にまとまる", async () => {
  await renderPanel(false);

  await emitCatalogChanged(3);

  expect(api.listSkippedFiles).toHaveBeenCalledTimes(2);
});

test("アンマウントで購読を解除し、保留中の取得も行わない", async () => {
  const { unmount } = await renderPanel(false);

  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
  try {
    await act(async () => {
      emitCatalog!();
    });
    unmount();
    await act(async () => {
      vi.advanceTimersByTime(REFETCH_DELAY_MS);
    });
  } finally {
    vi.useRealTimers();
  }
  await settle();

  expect(unlisten).toHaveBeenCalledTimes(1);
  expect(api.listSkippedFiles).toHaveBeenCalledTimes(1);
});
