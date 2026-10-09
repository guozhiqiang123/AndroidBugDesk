import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  realpath,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createBuildManifest } from "../scripts/build-manifest.mjs";

const scriptPath = fileURLToPath(
  new URL("../scripts/build-manifest.mjs", import.meta.url),
);
const abcHash =
  "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const cases = [
  {
    target: "aarch64-apple-darwin",
    os: "macos",
    arch: "arm64",
    paths: ["Android Bug Desk.app.zip", "dmg/Android Bug Desk_arm64.dmg"],
  },
  {
    target: "x86_64-apple-darwin",
    os: "macos",
    arch: "x64",
    paths: ["Android Bug Desk.app.zip", "dmg/Android Bug Desk_x64.dmg"],
  },
  {
    target: "x86_64-pc-windows-msvc",
    os: "windows",
    arch: "x64",
    paths: ["nsis/Android Bug Desk_x64-setup.exe"],
  },
  {
    target: "x86_64-unknown-linux-gnu",
    os: "linux",
    arch: "x64",
    paths: [
      "deb/android-bug-desk_amd64.deb",
      "appimage/Android Bug Desk_amd64.AppImage",
    ],
  },
];

async function fixture(t, target = cases[0].target) {
  const projectDir = await realpath(
    await mkdtemp(join(tmpdir(), "build manifest 空格 ")),
  );
  t.after(() => rm(projectDir, { recursive: true, force: true }));
  const git = (args) =>
    execFileSync("git", args, {
      cwd: projectDir,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }).trim();
  git(["init", "--quiet"]);
  await writeFile(
    join(projectDir, "package.json"),
    JSON.stringify({ name: "android-bug-desk", version: "0.1.0" }),
  );
  git(["add", "package.json"]);
  git([
    "-c",
    "user.name=Build manifest fixture",
    "-c",
    "user.email=build-manifest@example.invalid",
    "-c",
    "commit.gpgsign=false",
    "commit",
    "--quiet",
    "-m",
    "Synthetic manifest fixture",
  ]);
  const bundleDir = join(
    projectDir,
    "src-tauri",
    "target",
    target,
    "release",
    "bundle",
  );
  const outputDir = join(projectDir, "candidate outputs");
  await mkdir(bundleDir, { recursive: true });
  async function put(path, bytes = "abc") {
    const absolute = join(bundleDir, path);
    await mkdir(dirname(absolute), { recursive: true });
    await writeFile(absolute, bytes);
  }
  async function packages(
    paths = cases.find((entry) => entry.target === target).paths,
  ) {
    for (const path of paths) await put(path);
  }
  return {
    projectDir,
    bundleDir,
    outputDir,
    target,
    commit: git(["rev-parse", "HEAD"]),
    put,
    packages,
  };
}

function cli(f, args = [], overrides = {}) {
  const env = { ...process.env };
  for (const name of [
    "GITHUB_SHA",
    "GITHUB_RUN_ID",
    "GITHUB_RUN_ATTEMPT",
    "GITHUB_OUTPUT",
  ])
    delete env[name];
  return spawnSync(
    process.execPath,
    [scriptPath, "--target", f.target, ...args],
    {
      cwd: f.projectDir,
      encoding: "utf8",
      env: { ...env, ...overrides },
    },
  );
}

for (const entry of cases) {
  test(`stages only expected formats with exact byte hashes: ${entry.target}`, async (t) => {
    const f = await fixture(t, entry.target);
    await f.packages();
    // Files in other target directories and executable build helpers must stay out.
    await f.put("helpers/unused.exe");
    await f.put("msi/unused.msi");
    await f.put("rpm/unused.rpm");
    await f.put("dmg/notes.txt");
    await f.put("macos/Android Bug Desk.app/Contents/MacOS/Android Bug Desk");
    const result = await createBuildManifest(f);
    assert.equal(result.manifest.source.commit, f.commit);
    assert.equal(result.manifest.source.trackedChanges, false);
    assert.deepEqual(result.manifest.build, {
      os: entry.os,
      arch: entry.arch,
      target: entry.target,
      hostPlatform: process.platform,
    });
    assert.equal(
      result.artifactName,
      `AndroidBugDesk-0.1.0-${f.commit}-${entry.target}`,
    );
    assert.deepEqual(
      result.manifest.files.map((file) => file.path).sort(),
      [...entry.paths].sort(),
    );
    for (const file of result.manifest.files) {
      assert.equal(file.sizeBytes, 3);
      assert.equal(file.sha256, abcHash);
      assert.equal(
        await readFile(join(result.artifactPath, file.path), "utf8"),
        "abc",
      );
      assert.equal(await readFile(join(f.bundleDir, file.path), "utf8"), "abc");
    }
    const serialized = await readFile(result.manifestPath);
    assert.deepEqual(JSON.parse(serialized), result.manifest);
    const checksumLines = (
      await readFile(join(result.artifactPath, "SHA256SUMS"), "utf8")
    )
      .trimEnd()
      .split("\n");
    assert.equal(checksumLines.length, entry.paths.length + 1);
    for (const line of checksumLines) {
      const sha256 = line.slice(0, 64);
      assert.equal(line.slice(64, 66), "  ");
      const bytes = await readFile(join(result.artifactPath, line.slice(66)));
      assert.equal(createHash("sha256").update(bytes).digest("hex"), sha256);
    }
    assert.deepEqual(await readdir(f.outputDir), [result.artifactName]);
  });
}

test("includes multiple packages of a format and hashes binary bytes", async (t) => {
  const f = await fixture(t, cases[2].target);
  await f.packages();
  const bytes = Buffer.from([0, 255, 10, 128, 13, 0]);
  await f.put("nsis/另一个 setup.exe", bytes);
  const { manifest } = await createBuildManifest(f);
  assert.equal(manifest.files.length, 2);
  const binary = manifest.files.find(
    (file) => file.path === "nsis/另一个 setup.exe",
  );
  assert.equal(binary.sizeBytes, bytes.length);
  assert.equal(binary.sha256, createHash("sha256").update(bytes).digest("hex"));
});

test("each missing required format fails even if other packages exist", async (t) => {
  for (const entry of cases) {
    for (const missing of entry.paths) {
      const f = await fixture(t, entry.target);
      await f.packages(entry.paths.filter((path) => path !== missing));
      await f.put("helpers/unrelated.exe");
      await assert.rejects(
        createBuildManifest(f),
        /Missing expected .* package/u,
      );
      assert.deepEqual(await readdir(f.outputDir), []);
    }
  }
});

test("wrong directories and wrong extension case do not satisfy a format", async (t) => {
  const f = await fixture(t, cases[3].target);
  await f.put("deb/app.deb");
  await f.put("appimage/app.appimage");
  await f.put("helpers/app.AppImage");
  await assert.rejects(
    createBuildManifest(f),
    /Missing expected appimage package/u,
  );
});

test("zero-byte packages fail and leave no partial candidate output", async (t) => {
  const f = await fixture(t, cases[2].target);
  await f.packages();
  await f.put("nsis/empty.exe", "");
  await assert.rejects(createBuildManifest(f), /Package file is empty/u);
  assert.deepEqual(await readdir(f.outputDir), []);
});

test("cannot overwrite an existing candidate with changed bytes", async (t) => {
  const f = await fixture(t, cases[2].target);
  await f.packages();
  const first = await createBuildManifest(f);
  await f.put(cases[2].paths[0], "changed bytes");
  await assert.rejects(
    createBuildManifest(f),
    /Candidate output already exists/u,
  );
  assert.equal(
    await readFile(join(first.artifactPath, cases[2].paths[0]), "utf8"),
    "abc",
  );
});

test("unknown targets, missing checkout and unborn Git HEAD fail", async (t) => {
  const f = await fixture(t);
  await assert.rejects(
    createBuildManifest({ ...f, target: "unknown" }),
    /Unsupported build target/u,
  );
  const empty = await mkdtemp(join(tmpdir(), "unknown commit "));
  t.after(() => rm(empty, { recursive: true, force: true }));
  await assert.rejects(
    createBuildManifest({ ...f, projectDir: empty }),
    /Cannot resolve a real Git/u,
  );
  execFileSync("git", ["init", "--quiet"], { cwd: empty });
  await assert.rejects(
    createBuildManifest({ ...f, projectDir: empty }),
    /Cannot resolve a real Git/u,
  );
});

test("mismatched CI commit and dirty CI checkout fail", async (t) => {
  const f = await fixture(t);
  await f.packages();
  await assert.rejects(
    createBuildManifest({ ...f, expectedCommit: "0".repeat(40) }),
    /does not match/u,
  );
  await writeFile(
    join(f.projectDir, "package.json"),
    JSON.stringify({ name: "android-bug-desk", version: "0.1.1" }),
  );
  await assert.rejects(
    createBuildManifest({ ...f, expectedCommit: f.commit }),
    /CI checkout has tracked changes/u,
  );
  assert.equal(
    (await createBuildManifest(f)).manifest.source.trackedChanges,
    true,
  );
});

test("unsafe version and incomplete CI provenance fail", async (t) => {
  const f = await fixture(t);
  await f.packages();
  await assert.rejects(
    createBuildManifest({ ...f, runId: "123" }),
    /must both be positive integers/u,
  );
  await assert.rejects(
    createBuildManifest({ ...f, runId: "123", runAttempt: "0" }),
    /must both be positive integers/u,
  );
  await writeFile(
    join(f.projectDir, "package.json"),
    JSON.stringify({ name: "android-bug-desk", version: "unknown/version\n" }),
  );
  await assert.rejects(createBuildManifest(f), /filename-safe version/u);
});

test("hidden files, symlinks and directories cannot masquerade as packages", async (t) => {
  const f = await fixture(t, cases[2].target);
  await f.put("nsis/.hidden.exe");
  await assert.rejects(
    createBuildManifest(f),
    /non-hidden regular package file/u,
  );
  await rm(join(f.bundleDir, "nsis/.hidden.exe"));
  // Windows file symlinks require privileges unrelated to the package checks.
  if (process.platform !== "win32") {
    await f.put("source bytes.txt");
    await symlink(
      join(f.bundleDir, "source bytes.txt"),
      join(f.bundleDir, "nsis/link.exe"),
    );
    await assert.rejects(
      createBuildManifest(f),
      /non-hidden regular package file/u,
    );
    await rm(join(f.bundleDir, "nsis/link.exe"));
  }
  await mkdir(join(f.bundleDir, "nsis/fake.exe"));
  await assert.rejects(
    createBuildManifest(f),
    /non-hidden regular package file/u,
  );
});

test("output overlapping the bundle is rejected", async (t) => {
  const f = await fixture(t);
  await f.packages();
  await assert.rejects(
    createBuildManifest({ ...f, outputDir: f.bundleDir }),
    /must not contain each other/u,
  );
  await assert.rejects(
    createBuildManifest({ ...f, outputDir: f.projectDir }),
    /must not contain each other/u,
  );
});

test("CLI uses the standard target path and emits real CI artifact outputs", async (t) => {
  const f = await fixture(t, cases[2].target);
  await f.packages();
  const githubOutput = join(f.projectDir, "GitHub outputs.txt");
  const run = cli(f, [], {
    GITHUB_SHA: f.commit,
    GITHUB_RUN_ID: "12345",
    GITHUB_RUN_ATTEMPT: "2",
    GITHUB_OUTPUT: githubOutput,
  });
  assert.equal(run.status, 0, run.stderr);
  const outputs = Object.fromEntries(
    (await readFile(githubOutput, "utf8"))
      .trimEnd()
      .split("\n")
      .map((line) => {
        const split = line.indexOf("=");
        return [line.slice(0, split), line.slice(split + 1)];
      }),
  );
  assert.equal(
    outputs.artifact_name,
    `AndroidBugDesk-0.1.0-${f.commit}-${f.target}-run12345-attempt2`,
  );
  assert.equal(
    outputs.artifact_path,
    join(f.projectDir, "artifacts/candidates", outputs.artifact_name),
  );
  const manifest = JSON.parse(await readFile(outputs.manifest_path, "utf8"));
  assert.deepEqual(manifest.ci, { runId: "12345", runAttempt: "2" });
  assert.equal(manifest.source.commit, f.commit);
  assert.match(run.stdout, /1 packages/u);
});

test("CLI handles explicit paths with spaces and fails without emitting outputs", async (t) => {
  const f = await fixture(t);
  await f.packages();
  const run = cli(f, [
    "--bundle-dir",
    f.bundleDir,
    "--output-dir",
    f.outputDir,
  ]);
  assert.equal(run.status, 0, run.stderr);
  const githubOutput = join(f.projectDir, "failed outputs.txt");
  const failure = cli(f, ["--unexpected", "value"], {
    GITHUB_OUTPUT: githubOutput,
  });
  assert.equal(failure.status, 1);
  assert.match(failure.stderr, /Usage:/u);
  await assert.rejects(readFile(githubOutput), { code: "ENOENT" });
  await rm(join(f.bundleDir, cases[0].paths[1]));
  const missing = cli(f, [], { GITHUB_OUTPUT: githubOutput });
  assert.equal(missing.status, 1);
  assert.match(missing.stderr, /Missing expected dmg/u);
  await assert.rejects(readFile(githubOutput), { code: "ENOENT" });
});
