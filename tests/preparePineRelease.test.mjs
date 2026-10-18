import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { prepareRelease, VERSION } from "../scripts/prepare-pine-release.mjs";

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "pine-release-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  fs.mkdirSync(path.join(root, "src-tauri"));
  fs.writeFileSync(path.join(root, "package.json"), JSON.stringify({ name: "codex-switcher", version: "0.2.20", dependencies: { example: "1.0.0" } }));
  fs.writeFileSync(path.join(root, "src-tauri/tauri.conf.json"), JSON.stringify({ version: "0.2.20", identifier: "com.lampese.codex-switcher", app: { windows: [{ title: "Codex Switcher", width: 900 }] }, bundle: { targets: "all", icon: ["icon.ico"], createUpdaterArtifacts: true }, plugins: { updater: { pubkey: "upstream", endpoints: ["https://example.invalid/latest.json"] } } }));
  fs.writeFileSync(path.join(root, "src-tauri/Cargo.toml"), '[package]\nname = "codex-switcher"\nversion = "0.2.20"\n[dependencies]\nexample = "1.0.0"\n');
  fs.writeFileSync(path.join(root, "src-tauri/Cargo.lock"), 'version = 4\n\n[[package]]\nname = "codex-switcher"\nversion = "0.2.20"\n\n[[package]]\nname = "example"\nversion = "0.2.20"\n');
  return root;
}

test("prepares four consistent release manifests without changing dependencies", (t) => {
  const root = fixture(t);
  assert.equal(prepareRelease(root).length, 4);
  const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
  assert.equal(pkg.version, VERSION);
  assert.deepEqual(pkg.dependencies, { example: "1.0.0" });
  const lock = fs.readFileSync(path.join(root, "src-tauri/Cargo.lock"), "utf8");
  assert.ok(lock.includes(`name = "codex-switcher"\nversion = "${VERSION}"`));
  assert.ok(lock.includes('name = "example"\nversion = "0.2.20"'));
});

test("fork installer cannot use the upstream update feed or identity", (t) => {
  const root = fixture(t);
  prepareRelease(root);
  const config = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
  assert.equal(config.identifier, "com.pine24.codex-switcher");
  assert.equal(config.productName, "Codex Switcher Pine");
  assert.equal(config.bundle.createUpdaterArtifacts, false);
  assert.deepEqual(config.plugins.updater, { pubkey: "", endpoints: [] });
  assert.deepEqual(config.bundle.targets, ["nsis"]);
  assert.equal(config.bundle.windows.nsis.installMode, "currentUser");
  assert.equal(config.app.windows[0].width, 900);
});

test("unexpected version fails before any files are written", (t) => {
  const root = fixture(t);
  const configPath = path.join(root, "src-tauri/tauri.conf.json");
  fs.writeFileSync(configPath, '{"version":"9.9.9"}');
  const oldPackage = fs.readFileSync(path.join(root, "package.json"), "utf8");
  assert.throws(() => prepareRelease(root));
  assert.equal(fs.readFileSync(path.join(root, "package.json"), "utf8"), oldPackage);
});

test("a missing root lockfile package does not rewrite an unrelated dependency", (t) => {
  const root = fixture(t);
  fs.writeFileSync(path.join(root, "src-tauri/Cargo.lock"), '[[package]]\nname = "other"\nversion = "0.2.20"\n');
  assert.throws(() => prepareRelease(root), /Expected one Cargo.lock/);
  assert.equal(JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version, "0.2.20");
});

test("Windows CRLF lockfiles retain unrelated contents", (t) => {
  const root = fixture(t);
  const name = path.join(root, "src-tauri/Cargo.lock");
  fs.writeFileSync(name, fs.readFileSync(name, "utf8").replaceAll("\n", "\r\n"));
  prepareRelease(root);
  assert.ok(fs.readFileSync(name, "utf8").includes(`name = "codex-switcher"\r\nversion = "${VERSION}"`));
});
