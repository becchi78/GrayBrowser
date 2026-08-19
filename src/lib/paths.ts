// Splits a full file path into "everything up to and including
// the last path separator" (dir) and "everything after it" (name), for
// DuplicateGroupsPanel/GenerationFailuresPanel's `-path` row -- App.css's
// `.file-path-dir`/`.file-path-file` then let `dir` shrink/ellipsis while
// `name` (the file name itself) always keeps its full intrinsic width.
//
// Deliberately takes only `filePath`, not a separate `fileName` to split
// against. An earlier version of this function took both and used
// `filePath.endsWith(fileName)` to find the split point, falling back to
// `{ dir: filePath, name: "" }` when they didn't match -- but that fallback
// silently reproduces the exact bug this function exists to fix (the file
// name disappearing from view). Windows path/file-name comparisons are
// case-insensitive, so a `file_path`/`file_name` pair that differs only in
// case (or in `/` vs `\` separators) would hit that fallback in real data,
// not just in theory. Splitting on the last separator character instead
// needs no cross-check against a second string, so this class of mismatch
// can't arise at all.
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
