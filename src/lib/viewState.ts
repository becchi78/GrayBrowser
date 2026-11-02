import type { SavedViewState, SortDirection, SortField } from "../types";

/** App の絞り込み・ソート状態。保存形式(SavedViewState)との相互変換に使う。 */
export interface ViewState {
  search: string;
  folderPath: string | null;
  sortField: SortField;
  sortDirection: SortDirection;
  tagIds: number[];
  ratings: number[];
}

export const INITIAL_VIEW_STATE: ViewState = {
  search: "",
  folderPath: null,
  sortField: "created_at",
  sortDirection: "asc",
  tagIds: [],
  ratings: [],
};

export function toSaved(state: ViewState): SavedViewState {
  return {
    search: state.search,
    folder_path: state.folderPath,
    sort_field: state.sortField,
    sort_direction: state.sortDirection,
    tag_ids: state.tagIds,
    ratings: state.ratings,
  };
}

export function fromSaved(saved: SavedViewState): ViewState {
  return {
    search: saved.search,
    folderPath: saved.folder_path,
    sortField: saved.sort_field,
    sortDirection: saved.sort_direction,
    tagIds: saved.tag_ids,
    ratings: saved.ratings,
  };
}

function sameArray(a: number[], b: number[]): boolean {
  return a.length === b.length && a.every((v, i) => v === b[i]);
}

/** 並びも含めて同じか。変化がなければ保存を省くための判定。 */
export function isSameViewState(a: ViewState, b: ViewState): boolean {
  return (
    a.search === b.search &&
    a.folderPath === b.folderPath &&
    a.sortField === b.sortField &&
    a.sortDirection === b.sortDirection &&
    sameArray(a.tagIds, b.tagIds) &&
    sameArray(a.ratings, b.ratings)
  );
}
