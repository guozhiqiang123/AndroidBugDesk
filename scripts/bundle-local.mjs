import { spawn } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import {
  mkdir,
  open,
  readFile,
  readdir,
  rmdir,
  stat,
  unlink,
  writeFile,
} from "node:fs/promises";
import { dirname, join, relative, resolve, isAbsolute } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { sourceSnapshot, assertSourceUnchanged } from "./workflow-source.mjs";

const projectRoot = fileURLToPath(new URL("..", import.meta.url));
const notaryVariables = [
  "APPLE_ID",
  "APPLE_PASSWORD",
  "APPLE_TEAM_ID",
  "APPLE_API_KEY",
  "APPLE_API_ISSUER",
  "APPLE_API_KEY_PATH",
  "APPLE_CERTIFICATE",
  "APPLE_CERTIFICATE_PASSWORD",
];

export async function runCommand(command, args, { cwd, env, logPath }) {
  const log = await open(logPath, "w");
  try {
    await new Promise((accept, reject) => {
      const child = spawn(command, args, {
        cwd,
        env,
        stdio: ["ignore", log.fd, log.fd],
      });
      child.once("error", reject);
      child.once("exit", (code, signal) =>
        code === 0
          ? accept()
          : reject(
              new Error(`${command} exited ${code ?? signal}; see ${logPath}`),
            ),
      );
    });
  } finally {
    await log.close();
  }
  return readFile(logPath, "utf8");
}

export async function applicationFiles(root) {
  const files = [];
  async function visit(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isSymbolicLink())
        throw new Error(`Unexpected symlink in application: ${path}`);
      if (entry.isDirectory()) await visit(path);
      else
        files.push({
          path: relative(root, path),
          sha256: createHash("sha256")
            .update(await readFile(path))
            .digest("hex"),
        });
    }
  }
  await visit(root);
  return files.sort((a, b) => a.path.localeCompare(b.path));
}

export async function bundleLocalMac({
  projectDir = projectRoot,
  platform = process.platform,
  arch = process.arch,
  outputDir,
  fromBuild,
  run = runCommand,
  progress = console.log,
} = {}) {
  if (platform !== "darwin" || !["arm64", "x64"].includes(arch)) {
    throw new Error(
      "bundle:local currently supports native macOS arm64/x64 only; use the existing platform/Actions workflow elsewhere.",
    );
  }
  projectDir = resolve(projectDir);
  const configPath = join(projectDir, "src-tauri/tauri.local-macos.conf.json");
  const localConfig = JSON.parse(await readFile(configPath, "utf8"));
  if (localConfig.bundle?.macOS?.signingIdentity !== "-")
    throw new Error(
      "Local macOS builds must use the checked-in ad hoc signing configuration.",
    );
  const config = JSON.parse(
    await readFile(join(projectDir, "src-tauri/tauri.conf.json"), "utf8"),
  );
  const { productName, version, identifier } = config;
  if (
    ![productName, version, identifier].every(
      (s) => typeof s === "string" && s && !/[\r\n\0/\\]/u.test(s),
    )
  )
    throw new Error("Invalid package identity");
  const artifacts = join(projectDir, "artifacts");
  outputDir = resolve(
    projectDir,
    outputDir ??
      join(
        artifacts,
        `local-macos-${arch}-${new Date().toISOString().replace(/[:.]/g, "-")}-${randomUUID().slice(0, 8)}`,
      ),
  );
  const outputRelative = relative(artifacts, outputDir);
  if (
    !outputRelative ||
    outputRelative.startsWith("..") ||
    isAbsolute(outputRelative)
  )
    throw new Error("Output must be a new directory inside project artifacts/");
  await mkdir(dirname(outputDir), { recursive: true });
  await mkdir(outputDir); // Never overwrite another package or its receipt.
  const env = { ...process.env, APPLE_SIGNING_IDENTITY: "-" };
  for (const variable of notaryVariables) delete env[variable];
  const receipt = {
    passed: false,
    installed: false,
    architecture: arch,
    version,
    identifier,
    signingIdentity: "-",
    nodeVersion: process.version,
    phases: [],
    outputDir,
  };
  const receiptPath = join(outputDir, "bundle-receipt.json");
  const started = performance.now();
  async function phase(name, command, args) {
    progress(name);
    const start = performance.now();
    const entry = { name, command, args, passed: false, durationMs: 0 };
    receipt.phases.push(entry);
    try {
      const result = await run(command, args, {
        cwd: projectDir,
        env,
        logPath: join(outputDir, `${name}.log`),
      });
      entry.passed = true;
      return result;
    } finally {
      entry.durationMs = Math.round(performance.now() - start);
    }
  }
  try {
    const snapshot = await sourceSnapshot(projectDir);
    await writeFile(
      join(outputDir, "source-manifest.json"),
      JSON.stringify(snapshot, null, 2) + "\n",
    );
    const rust = await phase("preflight-rust", "rustc", ["-vV"]);
    receipt.rustVersion = rust;
    const host =
      arch === "arm64" ? "aarch64-apple-darwin" : "x86_64-apple-darwin";
    if (!rust.includes(`host: ${host}`))
      throw new Error(
        "Node and Rust host architectures differ; use native matching toolchains to reuse the native build cache.",
      );
    await phase("preflight-signing", "xcrun", ["--find", "codesign"]);
    const cli = join(projectDir, "node_modules/@tauri-apps/cli/tauri.js");
    const options = ["--config", configPath, "--ci"];
    const bundleRoot = join(projectDir, "src-tauri/target/release/bundle");
    const executable = join(
      projectDir,
      "src-tauri/target/release/android-bug-desk",
    );
    // Compile once. Packaging repairs can reuse a verified, unchanged compilation.
    if (fromBuild) {
      const priorDir = resolve(projectDir, fromBuild);
      const priorRelative = relative(artifacts, priorDir);
      if (
        !priorRelative ||
        priorRelative.startsWith("..") ||
        isAbsolute(priorRelative)
      )
        throw new Error("Build receipt must be inside project artifacts/");
      const priorPath = join(priorDir, "bundle-receipt.json");
      const prior = JSON.parse(await readFile(priorPath, "utf8"));
      if (
        prior.architecture !== arch ||
        prior.identifier !== identifier ||
        prior.version !== version ||
        !prior.phases.some(
          (p) =>
            ["build-app", "bundle-existing-app"].includes(p.name) && p.passed,
        ) ||
        !["verify-app-signature", "verify-architecture"].every((name) =>
          prior.phases.some((p) => p.name === name && p.passed),
        )
      ) {
        throw new Error("Prior receipt has no verified matching compilation");
      }
      await assertSourceUnchanged(
        projectDir,
        JSON.parse(
          await readFile(join(priorDir, "source-manifest.json"), "utf8"),
        ),
      );
      const fingerprint = createHash("sha256")
        .update(await readFile(executable))
        .digest("hex");
      if (prior.compiledExecutableSha256) {
        if (fingerprint !== prior.compiledExecutableSha256)
          throw new Error("Compiled executable changed since prior receipt");
      } else {
        // Initial workflow receipts predate the executable hash. Accept only the
        // binary written inside that recorded build's time window; no source drift.
        const builtAt = (await stat(executable)).mtimeMs;
        if (
          builtAt <
            (await stat(join(priorDir, "source-manifest.json"))).mtimeMs ||
          builtAt > (await stat(priorPath)).mtimeMs
        )
          throw new Error(
            "Cannot attribute compiled executable to prior build",
          );
      }
      const priorRust =
        prior.rustVersion ??
        (await readFile(join(priorDir, "preflight-rust.log"), "utf8"));
      if (
        priorRust !== rust ||
        (prior.nodeVersion && prior.nodeVersion !== process.version)
      )
        throw new Error("Toolchain changed; rebuild required");
      receipt.compilationReusedFrom = priorDir;
      await phase("bundle-existing-app", process.execPath, [
        cli,
        "bundle",
        "--bundles",
        "app",
        ...options,
      ]);
    } else {
      await phase("build-app", process.execPath, [
        cli,
        "build",
        "--bundles",
        "app",
        ...options,
        "--",
        "--locked",
      ]);
    }
    const app = join(bundleRoot, "macos", `${productName}.app`);
    await phase("verify-app-signature", "codesign", [
      "--verify",
      "--deep",
      "--strict",
      app,
    ]);
    const detectedArch = (
      await phase("verify-architecture", "lipo", [
        "-archs",
        join(app, "Contents/MacOS/android-bug-desk"),
      ])
    ).trim();
    if (detectedArch !== arch)
      throw new Error(`Expected ${arch}, got ${detectedArch}`);
    await assertSourceUnchanged(projectDir, snapshot);
    const dmgDirectory = join(bundleRoot, "dmg");
    try {
      for (const entry of await readdir(dmgDirectory, {
        withFileTypes: true,
      })) {
        if (
          entry.isFile() &&
          entry.name.startsWith(`${productName}_${version}_`) &&
          entry.name.endsWith(".dmg")
        ) {
          await unlink(join(dmgDirectory, entry.name));
        }
      }
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    await phase("bundle-dmg", process.execPath, [
      cli,
      "bundle",
      "--bundles",
      "app,dmg", // A dmg-only bundle deletes its temporary .app afterwards.
      ...options,
    ]);
    await phase("verify-final-app-signature", "codesign", [
      "--verify",
      "--deep",
      "--strict",
      app,
    ]);
    const fresh = [];
    for (const entry of await readdir(dmgDirectory, { withFileTypes: true })) {
      if (
        !entry.isFile() ||
        !entry.name.startsWith(`${productName}_${version}_`) ||
        !entry.name.endsWith(".dmg")
      )
        continue;
      const path = join(dmgDirectory, entry.name);
      fresh.push(path);
    }
    if (fresh.length !== 1)
      throw new Error(
        `Expected exactly one newly generated DMG, found ${fresh.length}`,
      );
    const packagePath = join(
      outputDir,
      `${productName}_${version}_macos-${arch}.dmg`,
    );
    await phase("stage-dmg", "ditto", [fresh[0], packagePath]);
    await phase("verify-dmg", "hdiutil", ["verify", packagePath]);
    const stagedApp = join(outputDir, `${productName}.app`);
    await phase("stage-app", "ditto", [app, stagedApp]);
    const appFiles = await applicationFiles(stagedApp);
    const mount = join(outputDir, "dmg-mount");
    await mkdir(mount);
    await phase("mount-dmg", "hdiutil", [
      "attach",
      "-readonly",
      "-nobrowse",
      "-mountpoint",
      mount,
      packagePath,
    ]);
    try {
      const packagedApp = join(mount, `${productName}.app`);
      await phase("verify-packaged-signature", "codesign", [
        "--verify",
        "--deep",
        "--strict",
        packagedApp,
      ]);
      if (
        JSON.stringify(await applicationFiles(packagedApp)) !==
        JSON.stringify(appFiles)
      )
        throw new Error("DMG application differs from staged application");
    } finally {
      await phase("unmount-dmg", "hdiutil", ["detach", mount]);
      await rmdir(mount);
    }
    await assertSourceUnchanged(projectDir, snapshot);
    const bytes = await readFile(packagePath);
    await writeFile(
      join(outputDir, "application-files.json"),
      JSON.stringify(appFiles, null, 2) + "\n",
    );
    Object.assign(receipt, {
      passed: true,
      sourceUnchanged: true,
      packagePath,
      stagedApp,
      sha256: createHash("sha256").update(bytes).digest("hex"),
      sizeBytes: bytes.length,
    });
    progress(`Verified local package: ${packagePath}`);
    return receipt;
  } catch (error) {
    receipt.error = String(error);
    throw error;
  } finally {
    const compilationVerified =
      receipt.phases.some(
        (p) =>
          ["build-app", "bundle-existing-app"].includes(p.name) && p.passed,
      ) &&
      receipt.phases.some((p) => p.name === "verify-architecture" && p.passed);
    if (compilationVerified) {
      const executable = join(
        projectDir,
        "src-tauri/target/release/android-bug-desk",
      );
      receipt.compiledExecutableSha256 = createHash("sha256")
        .update(await readFile(executable))
        .digest("hex");
    }
    receipt.totalDurationMs = Math.round(performance.now() - started);
    await writeFile(receiptPath, JSON.stringify(receipt, null, 2) + "\n");
  }
}

if (
  process.argv[1] &&
  pathToFileURL(resolve(process.argv[1])).href === import.meta.url
) {
  const args = process.argv.slice(2);
  const options = {};
  const keys = { "--output": "outputDir", "--from-build": "fromBuild" };
  let valid = args.length % 2 === 0;
  for (let index = 0; index < args.length; index += 2) {
    const key = keys[args[index]];
    if (!key || options[key] !== undefined || !args[index + 1]) valid = false;
    else options[key] = args[index + 1];
  }
  if (!valid) {
    console.error(
      "Usage: npm run bundle:local -- [--output artifacts/new-directory] [--from-build artifacts/previous-build]",
    );
    process.exitCode = 1;
  } else {
    await bundleLocalMac(options).catch((error) => {
      console.error(error.message);
      process.exitCode = 1;
    });
  }
}
