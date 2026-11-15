import type { ReactElement, ReactNode } from "react";
import { render, type RenderResult } from "@testing-library/react";
import userEvent, { type UserEvent } from "@testing-library/user-event";
import { UnreachableFoldersContext } from "../reachabilityContext";

interface Options {
  /** 到達できない監視フォルダ。省略時は「到達不能なし」(Context の既定値と同じ)。 */
  unreachableFolders?: readonly string[];
}

/**
 * 必要な Context を包んで描画し、userEvent も返す。
 * wrapper として渡すので、rerender した要素にも同じ Provider が自動で付く。
 */
export function renderWithProviders(
  ui: ReactElement,
  { unreachableFolders = [] }: Options = {},
): RenderResult & { user: UserEvent } {
  const user = userEvent.setup();
  const result = render(ui, {
    wrapper: ({ children }: { children: ReactNode }) => (
      <UnreachableFoldersContext.Provider value={unreachableFolders}>
        {children}
      </UnreachableFoldersContext.Provider>
    ),
  });
  return { ...result, user };
}
