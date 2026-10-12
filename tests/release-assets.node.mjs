import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { stageReleaseAssets } from "../scripts/release-assets.mjs";

const commit = "1".repeat(40);
const targets = [
  {
    target: "aarch64-apple-darwin",
    os: "macos",
    arch: "arm64",
    files: [
      ["Android Bug Desk.app.zip", "app.zip"],
      ["dmg/Android Bug Desk_arm64.dmg", "dmg"],
    ],
  },
  {
    target: "x86_64-apple-darwin",
    os: "macos",
    arch: "x64",
    files: [
      ["Android Bug Desk.app.zip", "app.zip"],
      ["dmg/Android Bug Desk_x64.dmg", "dmg"],
    ],
  },
  {
    target: "x86_64-pc-windows-msvc",
    os: "windows",
    arch: "x64",
    files: [["nsis/Android Bug Desk_x64-setup.exe", "nsis"]],
  },
  {
    target: "x86_64-unknown-linux-gnu",
    os: "linux",
    arch: "x64",
    files: [
      ["deb/android-bug-desk_amd64.deb", "deb"],
      ["appimage/Android Bug Desk_amd64.AppImage", "appimage"],
    ],
  },
];

function sha(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}

async function fixture(t, mutate) {
  const root = await mkdtemp(join(tmpdir(), "release assets "));
  t.after(() => rm(root, { recursive: true, force: true }));
  const inputDir = join(root, "downloaded");
  const outputDir = join(root, "release");
  await mkdir(inputDir);
  for (const specification of targets) {
    const artifactName = `AndroidBugDesk-0.1.0-${commit}-${specification.target}-run42-attempt1`;
    const artifactRoot = join(inputDir, artifactName);
    await mkdir(artifactRoot);
    const files = [];
    for (const [path, format] of specification.files) {
      const bytes = Buffer.from(`${specification.target}:${format}`);
      const absolute = join(artifactRoot, path);
      await mkdir(dirname(absolute), { recursive: true });
      await writeFile(absolute, bytes);
      files.push({ path, format, sizeBytes: bytes.length, sha256: sha(bytes) });
    }
    const manifest = {
      schemaVersion: 1,
      application: { name: "android-bug-desk", version: "0.1.0" },
      source: { commit, trackedChanges: false },
      build: {
        os: specification.os,
        arch: specification.arch,
        target: specification.target,
        hostPlatform: "fixture",
      },
      ci: { runId: "42", runAttempt: "1" },
      createdAt: "2026-10-12T00:00:00.000Z",
      artifactName,
      files,
    };
    mutate?.(manifest, specification);
    const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`);
    await writeFile(join(artifactRoot, "build-manifest.json"), manifestBytes);
    const sums = [
      ...files.map((file) => `${file.sha256}  ${file.path}\n`),
      `${sha(manifestBytes)}  build-manifest.json\n`,
    ].join("");
    await writeFile(join(artifactRoot, "SHA256SUMS"), sums);
  }
  return { root, inputDir, outputDir };
}

test("verifies four platforms and stages deterministic official release names", async (t) => {
  const f = await fixture(t);
  const result = await stageReleaseAssets({
    inputDir: f.inputDir,
    outputDir: f.outputDir,
    tag: "v0.1.0",
    expectedCommit: commit,
  });
  assert.deepEqual(
    (await readdir(f.outputDir)).sort(),
    [
      "Android-Bug-Desk-0.1.0-linux-x64.AppImage",
      "Android-Bug-Desk-0.1.0-linux-x64.deb",
      "Android-Bug-Desk-0.1.0-macos-arm64.app.zip",
      "Android-Bug-Desk-0.1.0-macos-arm64.dmg",
      "Android-Bug-Desk-0.1.0-macos-x64.app.zip",
      "Android-Bug-Desk-0.1.0-macos-x64.dmg",
      "Android-Bug-Desk-0.1.0-windows-x64-setup.exe",
      "SHA256SUMS",
      "release-manifest.json",
    ].sort(),
  );
  assert.equal(result.releaseManifest.assets.length, 7);
  assert.equal(result.releaseManifest.source.commit, commit);
  const sums = await readFile(join(f.outputDir, "SHA256SUMS"), "utf8");
  for (const line of sums.trimEnd().split("\n")) {
    const bytes = await readFile(join(f.outputDir, line.slice(66)));
    assert.equal(sha(bytes), line.slice(0, 64));
  }
});

test("rejects a tag that does not match the packaged version", async (t) => {
  const f = await fixture(t);
  await assert.rejects(
    stageReleaseAssets({
      inputDir: f.inputDir,
      outputDir: f.outputDir,
      tag: "v0.2.0",
      expectedCommit: commit,
    }),
    /version mismatch/u,
  );
});

test("rejects tampered package bytes", async (t) => {
  const f = await fixture(t);
  const artifact = (await readdir(f.inputDir)).find((name) =>
    name.includes("x86_64-pc-windows-msvc"),
  );
  await writeFile(
    join(f.inputDir, artifact, "nsis/Android Bug Desk_x64-setup.exe"),
    "tampered",
  );
  await assert.rejects(
    stageReleaseAssets({
      inputDir: f.inputDir,
      outputDir: f.outputDir,
      tag: "v0.1.0",
      expectedCommit: commit,
    }),
    /package hash mismatch/u,
  );
});

test("rejects duplicate or missing target provenance", async (t) => {
  const f = await fixture(t, (manifest, specification) => {
    if (specification.target === "aarch64-apple-darwin") {
      manifest.build.target = "x86_64-apple-darwin";
      manifest.build.arch = "x64";
    }
  });
  await assert.rejects(
    stageReleaseAssets({
      inputDir: f.inputDir,
      outputDir: f.outputDir,
      tag: "v0.1.0",
      expectedCommit: commit,
    }),
    /Duplicate build target/u,
  );
});
