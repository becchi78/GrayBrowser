// Splits a full file path into "everything up to and including
// the last path separator" (dir) and "everything after it" (name), for
// DuplicateGroupsPanel/GenerationFailuresPanel's `-path` row -- App.css's
// `.file-path-dir`/`.file-path-file` then let `dir` shrink/ellipsis while
// `name` (the file name itself) always keeps its full intrinsic width.
//
// Deliberately takes only `filePath`, not a separate `fileName` to split
// against: Windows path/file-name comparisons are case-insensitive, so a
// `file_path`/`file_name` pair differing only in case (or in `/` vs `\`
// separators) would fail any cross-check between the two strings and could
// make the file name disappear from view. Splitting on the last separator
// character needs no second string, so this class of mismatch can't arise.
export function splitDirAndFileName(filePath: string): { dir: string; name: string } {
  const lastSep = Math.max(filePath.lastIndexOf("\\"), filePath.lastIndexOf("/"));
  if (lastSep === -1 || lastSep === filePath.length - 1) {
    // No separator found, or the string ends with one (e.g. a bare file
    // name, or a directory-only path -- neither expected from real
    // `videos.file_path` data, but handled defensively). Put the whole
    // string in `name` rather than leaving it empty: losing information is
    // exactly the failure mode this function exists to avoid, so the safe
    // default is "show everything as the name", not "show nothing".
    return { dir: "", name: filePath };
  }
  return { dir: filePath.slice(0, lastSep + 1), name: filePath.slice(lastSep + 1) };
}
