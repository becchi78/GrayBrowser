// Launches Vitest with its CLI entry and cwd spelled as the real on-disk path.
//
// When the shell's cwd uses a lowercase drive letter (`c:\...`), npm/npx pick a
// lowercase path for the Vitest CLI entry, and Vitest then fails every test
// file with "failed to find the current suite". Success depends only on how
// that entry path is spelled, and vitest.config.ts cannot influence it, so the
// entry is resolved to its canonical spelling before being handed to node.
import { spawnSync } from "node:child_process";
import { realpathSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const vitestEntry = realpathSync.native(join(root, "node_modules", "vitest", "vitest.mjs"));

const result = spawnSync(process.execPath, [vitestEntry, ...process.argv.slice(2)], {
  stdio: "inherit",
  cwd: realpathSync.native(process.cwd()),
});

// A null status means the child was killed by a signal or failed to spawn.
process.exit(result.status ?? 1);
