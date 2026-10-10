import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

// Documentation and test-only edits do not invalidate compiled application inputs.
export async function sourceSnapshot(projectDir, browser = false) {
  const paths = execFileSync(
    "git",
    ["ls-files", "-co", "--exclude-standard", "-z"],
    { cwd: projectDir, encoding: "utf8" },
  ).split("\0");
  const prefixes = browser
    ? ["src/", "tests/fixtures/", "scripts/"]
    : ["src/", "src-tauri/", "schemas/", "profiles/"];
  const roots = new Set([
    "package.json",
    "package-lock.json",
    "index.html",
    "vite.config.ts",
    "tsconfig.json",
  ]);
  const inputs = [...new Set(paths)]
    .filter(
      (p) =>
        p && (roots.has(p) || prefixes.some((prefix) => p.startsWith(prefix))),
    )
    .sort();
  return Promise.all(
    inputs.map(async (path) => ({
      path,
      sha256: createHash("sha256")
        .update(await readFile(join(projectDir, path)))
        .digest("hex"),
    })),
  );
}

export async function assertSourceUnchanged(
  projectDir,
  before,
  browser = false,
) {
  const after = await sourceSnapshot(projectDir, browser);
  if (JSON.stringify(before) !== JSON.stringify(after)) {
    const previous = new Map(before.map((entry) => [entry.path, entry.sha256]));
    const current = new Map(after.map((entry) => [entry.path, entry.sha256]));
    const changed = [...new Set([...previous.keys(), ...current.keys()])]
      .filter((path) => previous.get(path) !== current.get(path))
      .sort();
    throw new Error(
      `Source changed during verification: ${changed.join(", ")}; this result cannot validate the new source. Stop concurrent edits and rerun affected checks.`,
    );
  }
}
