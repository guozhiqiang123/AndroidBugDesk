import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
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
import {
  dirname,
  isAbsolute,
  join,
  posix,
  relative,
  resolve,
  sep,
} from "node:path";
import { pathToFileURL } from "node:url";

const expectedTargets = {
  "aarch64-apple-darwin": {
    os: "macos",
    arch: "arm64",
    formats: ["app.zip", "dmg"],
    names: {
      "app.zip": "macos-arm64.app.zip",
      dmg: "macos-arm64.dmg",
    },
  },
  "x86_64-apple-darwin": {
    os: "macos",
    arch: "x64",
    formats: ["app.zip", "dmg"],
    names: {
      "app.zip": "macos-x64.app.zip",
      dmg: "macos-x64.dmg",
    },
  },
  "x86_64-pc-windows-msvc": {
    os: "windows",
    arch: "x64",
    formats: ["nsis"],
    names: { nsis: "windows-x64-setup.exe" },
  },
  "x86_64-unknown-linux-gnu": {
    os: "linux",
    arch: "x64",
    formats: ["deb", "appimage"],
    names: {
      deb: "linux-x64.deb",
      appimage: "linux-x64.AppImage",
    },
  },
};

function within(parent, child) {
  const path = relative(parent, child);
  return (
    path === "" ||
    (!isAbsolute(path) && path !== ".." && !path.startsWith(`..${sep}`))
  );
}

function safeRelativePath(value) {
  if (
    typeof value !== "string" ||
    !value ||
    /[\\\r\n\0]/u.test(value) ||
    isAbsolute(value) ||
    posix.normalize(value) !== value ||
    value === ".." ||
    value.startsWith("../")
  ) {
    throw new Error(`Unsafe package path: ${String(value)}`);
  }
  return value;
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

async function requireRegularFile(path, description) {
  const stat = await lstat(path);
  if (!stat.isFile() || stat.isSymbolicLink()) {
    throw new Error(`Expected a regular ${description}: ${path}`);
  }
}

function requireString(value, description) {
  if (typeof value !== "string" || !value) {
    throw new Error(`Missing ${description}`);
  }
  return value;
}

function parseTag(tag) {
  const match =
    /^v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?)$/u.exec(tag);
  if (!match) throw new Error(`Release tag must be v<semver>: ${tag}`);
  return match[1];
}

function validateManifest(manifest, artifactName, version, expectedCommit) {
  if (manifest?.schemaVersion !== 1) {
    throw new Error(`${artifactName}: unsupported build manifest schema`);
  }
  if (
    manifest.application?.name !== "android-bug-desk" ||
    manifest.application?.version !== version
  ) {
    throw new Error(`${artifactName}: application or tag version mismatch`);
  }
  if (
    manifest.source?.commit !== expectedCommit ||
    manifest.source?.trackedChanges !== false
  ) {
    throw new Error(`${artifactName}: source commit is not release-safe`);
  }
  if (manifest.artifactName !== artifactName) {
    throw new Error(`${artifactName}: artifact directory name mismatch`);
  }
  const target = requireString(manifest.build?.target, "build target");
  const specification = expectedTargets[target];
  if (!specification) throw new Error(`${artifactName}: unexpected target`);
  if (
    manifest.build.os !== specification.os ||
    manifest.build.arch !== specification.arch
  ) {
    throw new Error(`${artifactName}: target platform metadata mismatch`);
  }
  if (!Array.isArray(manifest.files)) {
    throw new Error(`${artifactName}: files must be an array`);
  }
  return { target, specification };
}

async function validateChecksums(artifactRoot, manifest, manifestBytes) {
  const expected = [
    ...manifest.files.map((file) => `${file.sha256}  ${file.path}\n`),
    `${createHash("sha256").update(manifestBytes).digest("hex")}  build-manifest.json\n`,
  ].join("");
  const sumsPath = join(artifactRoot, "SHA256SUMS");
  await requireRegularFile(sumsPath, "checksum file");
  if ((await readFile(sumsPath, "utf8")) !== expected) {
    throw new Error(
      `${manifest.artifactName}: SHA256SUMS does not match manifest`,
    );
  }
}

export async function stageReleaseAssets({
  inputDir,
  outputDir,
  tag,
  expectedCommit,
}) {
  const version = parseTag(requireString(tag, "release tag"));
  if (
    !/^[0-9a-f]{40}$/u.test(expectedCommit ?? "") ||
    /^0+$/u.test(expectedCommit)
  ) {
    throw new Error("Expected commit must be a real 40-character Git commit");
  }
  inputDir = await realpath(
    resolve(requireString(inputDir, "input directory")),
  );
  outputDir = resolve(requireString(outputDir, "output directory"));
  if (within(inputDir, outputDir) || within(outputDir, inputDir)) {
    throw new Error("Input and output directories must not contain each other");
  }
  try {
    await lstat(outputDir);
    throw new Error(`Release output already exists: ${outputDir}`);
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }

  const entries = await readdir(inputDir, { withFileTypes: true });
  if (
    entries.length !== Object.keys(expectedTargets).length ||
    entries.some((entry) => !entry.isDirectory() || entry.isSymbolicLink())
  ) {
    throw new Error("Expected exactly four platform artifact directories");
  }

  await mkdir(dirname(outputDir), { recursive: true });
  const stagingDir = await mkdtemp(
    join(dirname(outputDir), ".release-assets-"),
  );
  try {
    const foundTargets = new Set();
    const releaseAssets = [];
    for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
      const artifactRoot = join(inputDir, entry.name);
      const manifestPath = join(artifactRoot, "build-manifest.json");
      await requireRegularFile(manifestPath, "build manifest");
      const manifestBytes = await readFile(manifestPath);
      let manifest;
      try {
        manifest = JSON.parse(manifestBytes);
      } catch {
        throw new Error(`${entry.name}: invalid build manifest JSON`);
      }
      const { target, specification } = validateManifest(
        manifest,
        entry.name,
        version,
        expectedCommit,
      );
      if (foundTargets.has(target)) {
        throw new Error(`Duplicate build target: ${target}`);
      }
      foundTargets.add(target);

      const formats = new Map();
      for (const file of manifest.files) {
        const path = safeRelativePath(file?.path);
        const format = requireString(file?.format, "package format");
        if (!specification.formats.includes(format) || formats.has(format)) {
          throw new Error(
            `${entry.name}: duplicate or unexpected package format`,
          );
        }
        if (
          !Number.isSafeInteger(file.sizeBytes) ||
          file.sizeBytes <= 0 ||
          !/^[0-9a-f]{64}$/u.test(file.sha256 ?? "")
        ) {
          throw new Error(`${entry.name}: invalid package fingerprint`);
        }
        const sourcePath = resolve(artifactRoot, ...path.split("/"));
        if (!within(artifactRoot, sourcePath)) {
          throw new Error(`${entry.name}: package escapes artifact directory`);
        }
        await requireRegularFile(sourcePath, "package file");
        const actual = await hashFile(sourcePath);
        if (
          actual.sizeBytes !== file.sizeBytes ||
          actual.sha256 !== file.sha256
        ) {
          throw new Error(`${entry.name}: package hash mismatch: ${path}`);
        }
        formats.set(format, { ...file, sourcePath });
      }
      if (
        specification.formats.some((format) => !formats.has(format)) ||
        formats.size !== specification.formats.length
      ) {
        throw new Error(`${entry.name}: missing required package format`);
      }
      await validateChecksums(artifactRoot, manifest, manifestBytes);

      for (const format of specification.formats) {
        const file = formats.get(format);
        const name = `Android-Bug-Desk-${version}-${specification.names[format]}`;
        const destination = join(stagingDir, name);
        await copyFile(file.sourcePath, destination);
        const copied = await hashFile(destination);
        if (
          copied.sha256 !== file.sha256 ||
          copied.sizeBytes !== file.sizeBytes
        ) {
          throw new Error(`${entry.name}: staged package verification failed`);
        }
        releaseAssets.push({
          name,
          target,
          os: specification.os,
          arch: specification.arch,
          format,
          ...copied,
        });
      }
    }
    for (const target of Object.keys(expectedTargets)) {
      if (!foundTargets.has(target))
        throw new Error(`Missing build target: ${target}`);
    }

    releaseAssets.sort((a, b) => a.name.localeCompare(b.name));
    const releaseManifest = {
      schemaVersion: 1,
      application: { name: "android-bug-desk", version },
      tag,
      source: { commit: expectedCommit },
      createdAt: new Date().toISOString(),
      assets: releaseAssets,
    };
    const releaseManifestPath = join(stagingDir, "release-manifest.json");
    await writeFile(
      releaseManifestPath,
      `${JSON.stringify(releaseManifest, null, 2)}\n`,
      { flag: "wx" },
    );
    const releaseManifestHash = await hashFile(releaseManifestPath);
    const sums = [
      ...releaseAssets,
      { name: "release-manifest.json", ...releaseManifestHash },
    ]
      .map((file) => `${file.sha256}  ${file.name}\n`)
      .join("");
    await writeFile(join(stagingDir, "SHA256SUMS"), sums, { flag: "wx" });
    await rename(stagingDir, outputDir);
    return { outputDir, releaseManifest };
  } catch (error) {
    await rm(stagingDir, { recursive: true, force: true });
    throw error;
  }
}

function parseArguments(args) {
  const options = {};
  const flags = {
    "--input": "inputDir",
    "--output": "outputDir",
    "--tag": "tag",
    "--commit": "expectedCommit",
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
        "Usage: node scripts/release-assets.mjs --input <dir> --output <dir> --tag <vX.Y.Z> --commit <sha>",
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
    const result = await stageReleaseAssets(
      parseArguments(process.argv.slice(2)),
    );
    process.stdout.write(
      `Release assets staged: ${result.outputDir} (${result.releaseManifest.assets.length} packages)\n`,
    );
  } catch (error) {
    process.stderr.write(`Release asset staging failed: ${error.message}\n`);
    process.exitCode = 1;
  }
}
