import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  appendFile,
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { pathToFileURL } from "node:url";

// These paths are the Tauri bundle directories, not a recursive extension glob.
const targets = {
  "aarch64-apple-darwin": {
    os: "macos",
    arch: "arm64",
    formats: [
      { directory: "", suffix: ".app.zip", format: "app.zip" },
      { directory: "dmg", suffix: ".dmg", format: "dmg" },
    ],
  },
  "x86_64-apple-darwin": {
    os: "macos",
    arch: "x64",
    formats: [
      { directory: "", suffix: ".app.zip", format: "app.zip" },
      { directory: "dmg", suffix: ".dmg", format: "dmg" },
    ],
  },
  "x86_64-pc-windows-msvc": {
    os: "windows",
    arch: "x64",
    formats: [{ directory: "nsis", suffix: ".exe", format: "nsis" }],
  },
  "x86_64-unknown-linux-gnu": {
    os: "linux",
    arch: "x64",
    formats: [
      { directory: "deb", suffix: ".deb", format: "deb" },
      { directory: "appimage", suffix: ".AppImage", format: "appimage" },
    ],
  },
};

function safePath(value) {
  if (/[\r\n\0]/u.test(value)) {
    throw new Error("Paths must not contain newlines or NUL bytes");
  }
  return value;
}

function within(parent, child) {
  const path = relative(parent, child);
  return (
    path === "" ||
    (!isAbsolute(path) && path !== ".." && !path.startsWith(`..${sep}`))
  );
}

function git(projectDir, args) {
  try {
    return execFileSync("git", args, {
      cwd: projectDir,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }).trim();
  } catch {
    throw new Error("Cannot resolve a real Git checkout and commit");
  }
}

async function hashFile(path) {
  const hash = createHash("sha256");
  let sizeBytes = 0;
  for await (const bytes of createReadStream(path)) {
    hash.update(bytes);
    sizeBytes += bytes.length;
  }
  return { sizeBytes, sha256: hash.digest("hex") };
}

async function findPackages(bundleDir, rule) {
  const directory = join(bundleDir, rule.directory);
  let entries;
  try {
    const stat = await lstat(directory);
    if (!stat.isDirectory() || stat.isSymbolicLink()) {
      throw new Error(`Expected a real bundle directory: ${directory}`);
    }
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }

  const files = [];
  for (const entry of entries) {
    if (!entry.name.endsWith(rule.suffix)) continue;
    const path = safePath(join(rule.directory, entry.name));
    // A hidden package would be silently omitted by upload-artifact's defaults.
    if (
      entry.name.startsWith(".") ||
      !entry.isFile() ||
      entry.isSymbolicLink()
    ) {
      throw new Error(`Expected a non-hidden regular package file: ${path}`);
    }
    files.push({ path, format: rule.format });
  }
  return files.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
}

export async function createBuildManifest({
  target,
  projectDir = process.cwd(),
  bundleDir,
  outputDir,
  expectedCommit,
  runId,
  runAttempt,
}) {
  if (!Object.hasOwn(targets, target)) {
    throw new Error(`Unsupported build target: ${target}`);
  }
  const specification = targets[target];
  projectDir = await realpath(resolve(safePath(projectDir)));
  const checkoutRoot = await realpath(
    git(projectDir, ["rev-parse", "--show-toplevel"]),
  );
  if (checkoutRoot !== projectDir) {
    throw new Error("Project directory must be the Git checkout root");
  }
  const commit = git(projectDir, ["rev-parse", "--verify", "HEAD^{commit}"]);
  if (!/^[0-9a-f]{40}$/u.test(commit) || /^0+$/u.test(commit)) {
    throw new Error("Cannot resolve a real Git commit");
  }
  if (expectedCommit !== undefined && expectedCommit !== commit) {
    throw new Error("GITHUB_SHA does not match the checked-out Git commit");
  }
  const trackedChanges =
    git(projectDir, ["status", "--porcelain", "--untracked-files=no"]) !== "";
  if (expectedCommit !== undefined && trackedChanges) {
    throw new Error(
      "CI checkout has tracked changes; packages cannot be attributed to its commit",
    );
  }
  const application = JSON.parse(
    await readFile(join(projectDir, "package.json"), "utf8"),
  );
  if (
    typeof application.version !== "string" ||
    !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/u.test(
      application.version,
    )
  ) {
    throw new Error("package.json must contain a valid, filename-safe version");
  }
  if (typeof application.name !== "string" || !application.name.trim()) {
    throw new Error("package.json must contain an application name");
  }
  const hasRun = runId !== undefined || runAttempt !== undefined;
  if (
    hasRun &&
    (!/^[1-9]\d*$/u.test(runId ?? "") || !/^[1-9]\d*$/u.test(runAttempt ?? ""))
  ) {
    throw new Error("CI run ID and attempt must both be positive integers");
  }
  const artifactName = `AndroidBugDesk-${application.version}-${commit}-${target}${hasRun ? `-run${runId}-attempt${runAttempt}` : ""}`;
  bundleDir = await realpath(
    resolve(
      projectDir,
      safePath(bundleDir ?? `src-tauri/target/${target}/release/bundle`),
    ),
  );
  outputDir = resolve(
    projectDir,
    safePath(outputDir ?? "artifacts/candidates"),
  );
  await mkdir(outputDir, { recursive: true });
  outputDir = await realpath(outputDir);
  if (within(bundleDir, outputDir) || within(outputDir, bundleDir)) {
    throw new Error(
      "Output directory and bundle directory must not contain each other",
    );
  }
  const artifactPath = join(outputDir, artifactName);
  try {
    await lstat(artifactPath);
    throw new Error(`Candidate output already exists: ${artifactPath}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }

  const packages = [];
  for (const rule of specification.formats) {
    const files = await findPackages(bundleDir, rule);
    if (files.length === 0) {
      throw new Error(
        `Missing expected ${rule.format} package in ${rule.directory || "bundle root"} for ${target}`,
      );
    }
    packages.push(...files);
  }

  // Stage only validated formats. Hash the staged bytes that will be uploaded.
  const stagingDir = await mkdtemp(join(outputDir, ".build-manifest-"));
  try {
    const files = [];
    for (const file of packages) {
      const stagedPath = join(stagingDir, file.path);
      await mkdir(dirname(stagedPath), { recursive: true });
      await copyFile(join(bundleDir, file.path), stagedPath);
      const fingerprint = await hashFile(stagedPath);
      if (fingerprint.sizeBytes === 0) {
        throw new Error(`Package file is empty: ${file.path}`);
      }
      files.push({
        ...file,
        path: file.path.split(sep).join("/"),
        ...fingerprint,
      });
    }
    const manifest = {
      schemaVersion: 1,
      application: { name: application.name, version: application.version },
      source: { commit, trackedChanges },
      build: {
        os: specification.os,
        arch: specification.arch,
        target,
        hostPlatform: process.platform,
      },
      ...(hasRun ? { ci: { runId, runAttempt } } : {}),
      createdAt: new Date().toISOString(),
      artifactName,
      files,
    };
    const manifestPath = join(stagingDir, "build-manifest.json");
    await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, {
      flag: "wx",
    });
    const manifestHash = await hashFile(manifestPath);
    const sums = [...files, { path: "build-manifest.json", ...manifestHash }]
      .map((file) => `${file.sha256}  ${file.path}\n`)
      .join("");
    await writeFile(join(stagingDir, "SHA256SUMS"), sums, { flag: "wx" });
    await rename(stagingDir, artifactPath);
    return {
      artifactName,
      artifactPath,
      manifestPath: join(artifactPath, "build-manifest.json"),
      manifest,
    };
  } catch (error) {
    await rm(stagingDir, { recursive: true, force: true });
    throw error;
  }
}

function parseArguments(args) {
  const options = {};
  const flags = {
    "--target": "target",
    "--bundle-dir": "bundleDir",
    "--output-dir": "outputDir",
  };
  for (let index = 0; index < args.length; index += 2) {
    const key = Object.hasOwn(flags, args[index])
      ? flags[args[index]]
      : undefined;
    const value = args[index + 1];
    if (
      !key ||
      !value ||
      value.startsWith("--") ||
      Object.hasOwn(options, key)
    ) {
      throw new Error(
        "Usage: node scripts/build-manifest.mjs --target <Rust target> [--bundle-dir <path>] [--output-dir <path>]",
      );
    }
    options[key] = value;
  }
  return options;
}

if (
  process.argv[1] &&
  pathToFileURL(resolve(process.argv[1])).href === import.meta.url
) {
  try {
    const result = await createBuildManifest({
      ...parseArguments(process.argv.slice(2)),
      expectedCommit: process.env.GITHUB_SHA,
      runId: process.env.GITHUB_RUN_ID,
      runAttempt: process.env.GITHUB_RUN_ATTEMPT,
    });
    if (process.env.GITHUB_OUTPUT) {
      await appendFile(
        process.env.GITHUB_OUTPUT,
        `artifact_name=${result.artifactName}\nartifact_path=${result.artifactPath}\nmanifest_path=${result.manifestPath}\n`,
      );
    }
    process.stdout.write(
      `Manifest saved: ${result.manifestPath} (${result.manifest.files.length} packages)\n`,
    );
  } catch (error) {
    process.stderr.write(`Build manifest failed: ${error.message}\n`);
    process.exitCode = 1;
  }
}
