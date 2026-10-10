import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { bundleLocalMac } from "../scripts/bundle-local.mjs";

async function fixture(
  t,
  {
    failSignature = false,
    changeSource = false,
    mismatch = false,
    wrongHost = false,
  } = {},
) {
  const root = await mkdtemp(join(tmpdir(), "local bundle 空格 "));
  t.after(() => rm(root, { recursive: true, force: true }));
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  await mkdir(join(root, "src-tauri"));
  await writeFile(join(root, ".gitignore"), "artifacts/\nsrc-tauri/target/\n");
  await writeFile(join(root, "index.html"), "initial source");
  await writeFile(
    join(root, "src-tauri/tauri.conf.json"),
    JSON.stringify({
      productName: "Test App",
      version: "0.1.0",
      identifier: "test.local",
    }),
  );
  await cp(
    new URL("../src-tauri/tauri.local-macos.conf.json", import.meta.url),
    join(root, "src-tauri/tauri.local-macos.conf.json"),
  );
  const calls = [];
  const app = join(root, "src-tauri/target/release/bundle/macos/Test App.app");
  async function run(command, args, options) {
    calls.push({ command, args, env: options.env });
    await writeFile(options.logPath, "mock command\n");
    if (command === "rustc")
      return `host: ${wrongHost ? "x86_64" : "aarch64"}-apple-darwin\n`;
    if (args[1] === "build") {
      assert.equal(options.env.APPLE_SIGNING_IDENTITY, "-");
      assert.ok(args.includes("--locked"));
      assert.ok(
        args.includes(join(root, "src-tauri/tauri.local-macos.conf.json")),
      );
      await mkdir(join(app, "Contents/MacOS"), { recursive: true });
      await writeFile(
        join(app, "Contents/MacOS/android-bug-desk"),
        "signed executable",
      );
      await writeFile(
        join(root, "src-tauri/target/release/android-bug-desk"),
        "compiled executable",
      );
    }
    if (
      command === "codesign" &&
      calls.filter((c) => c.command === command).length === 1
    ) {
      if (failSignature) throw new Error("full app signature invalid");
      if (changeSource)
        await writeFile(join(root, "index.html"), "concurrent source change");
    }
    if (command === "lipo") return "arm64\n";
    if (args[1] === "bundle" && args.includes("app,dmg")) {
      const dir = join(root, "src-tauri/target/release/bundle/dmg");
      await mkdir(dir, { recursive: true });
      await writeFile(join(dir, "Test App_0.1.0_aarch64.dmg"), "synthetic DMG");
    }
    if (command === "ditto") await cp(args[0], args[1], { recursive: true });
    if (command === "hdiutil" && args[0] === "attach") {
      const mount = args[args.indexOf("-mountpoint") + 1];
      await cp(app, join(mount, "Test App.app"), { recursive: true });
      if (mismatch)
        await writeFile(
          join(mount, "Test App.app/extra.txt"),
          "different application",
        );
    }
    if (args[1] === "bundle" && args.includes("dmg"))
      await rm(app, { recursive: true, force: true });
    if (command === "hdiutil" && args[0] === "detach")
      await rm(join(args[1], "Test App.app"), { recursive: true });
    return "";
  }
  const options = {
    projectDir: root,
    platform: "darwin",
    arch: "arm64",
    outputDir: "artifacts/run",
    run,
    progress() {},
  };
  const receipt = async () =>
    JSON.parse(
      await readFile(join(root, "artifacts/run/bundle-receipt.json"), "utf8"),
    );
  return { root, calls, options, receipt };
}

test("correct local signing from the first build; compile once, verify before DMG and preserve receipt", async (t) => {
  const f = await fixture(t);
  const result = await bundleLocalMac(f.options);
  assert.equal(result.passed, true);
  assert.equal(result.installed, false);
  assert.equal(f.calls.filter((c) => c.args[1] === "build").length, 1);
  const signing = f.calls.findIndex((c) => c.command === "codesign");
  const dmg = f.calls.findIndex((c) => c.args[1] === "bundle");
  assert.ok(signing < dmg);
  assert.equal((await f.receipt()).sha256, result.sha256);
  assert.ok(result.phases.every((p) => p.passed && p.durationMs >= 0));
});

test("signature failure stops before generating DMG and records failure", async (t) => {
  const f = await fixture(t, { failSignature: true });
  await assert.rejects(bundleLocalMac(f.options), /signature invalid/);
  assert.ok(!f.calls.some((c) => c.args[1] === "bundle"));
  assert.equal((await f.receipt()).passed, false);
});

test("concurrent source edit stops before packaging", async (t) => {
  const f = await fixture(t, { changeSource: true });
  await assert.rejects(bundleLocalMac(f.options), /Source changed/);
  assert.ok(!f.calls.some((c) => c.args[1] === "bundle"));
});

test("package application mismatch fails and always detaches DMG", async (t) => {
  const f = await fixture(t, { mismatch: true });
  await assert.rejects(bundleLocalMac(f.options), /differs from staged/);
  assert.ok(
    f.calls.some((c) => c.command === "hdiutil" && c.args[0] === "detach"),
  );
  assert.equal((await f.receipt()).passed, false);
});

test("existing output is preserved", async (t) => {
  const f = await fixture(t);
  await mkdir(join(f.root, "artifacts/run"), { recursive: true });
  await writeFile(join(f.root, "artifacts/run/existing.txt"), "old package");
  await assert.rejects(bundleLocalMac(f.options), /EEXIST/);
  assert.equal(
    await readFile(join(f.root, "artifacts/run/existing.txt"), "utf8"),
    "old package",
  );
  assert.equal(f.calls.length, 0);
});

test("architecture mismatch and unsupported hosts fail before compiling", async (t) => {
  const f = await fixture(t, { wrongHost: true });
  await assert.rejects(bundleLocalMac(f.options), /architectures differ/);
  assert.ok(!f.calls.some((c) => c.args[1] === "build"));
  await assert.rejects(
    bundleLocalMac({ ...f.options, platform: "linux" }),
    /native macOS/,
  );
});

test("unchanged compilation can repair packaging without recompiling; changed binary is rejected", async (t) => {
  const f = await fixture(t);
  await bundleLocalMac(f.options);
  const initialBuilds = f.calls.filter((c) => c.args[1] === "build").length;
  const result = await bundleLocalMac({
    ...f.options,
    outputDir: "artifacts/resumed",
    fromBuild: "artifacts/run",
  });
  assert.equal(result.passed, true);
  assert.equal(result.compilationReusedFrom, join(f.root, "artifacts/run"));
  const chained = await bundleLocalMac({
    ...f.options,
    outputDir: "artifacts/resumed-again",
    fromBuild: "artifacts/resumed",
  });
  assert.equal(chained.passed, true);
  assert.equal(
    f.calls.filter((c) => c.args[1] === "build").length,
    initialBuilds,
  );
  await writeFile(
    join(f.root, "src-tauri/target/release/android-bug-desk"),
    "overwritten executable",
  );
  await assert.rejects(
    bundleLocalMac({
      ...f.options,
      outputDir: "artifacts/rejected",
      fromBuild: "artifacts/run",
    }),
    /executable changed/,
  );
});
