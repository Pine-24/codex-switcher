import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

const read = (name) => fs.readFileSync(new URL(`../${name}`, import.meta.url), "utf8");
const config = JSON.parse(read("src-tauri/tauri.conf.json"));

test("Pine release versions agree across all package manifests", () => {
  const version = "0.2.23-usage.1";
  assert.equal(JSON.parse(read("package.json")).version, version);
  assert.equal(config.version, version);
  assert.equal(read("src-tauri/Cargo.toml").match(/^version = "([^"]+)"/m)?.[1], version);
  assert.equal(read("src-tauri/Cargo.lock").match(/name = "codex-switcher"\r?\nversion = "([^"]+)"/)?.[1], version);
});

test("Pine retains its installer identity and cannot use upstream automatic updates", () => {
  assert.equal(config.productName, "Codex Switcher Pine");
  assert.equal(config.identifier, "com.pine24.codex-switcher");
  assert.equal(config.bundle.createUpdaterArtifacts, false);
  assert.deepEqual(config.plugins.updater, { pubkey: "", endpoints: [] });
  assert.equal(config.bundle.windows.nsis.installMode, "currentUser");
  for (const platform of ["windows", "linux"]) {
    const platformConfig = JSON.parse(read(`src-tauri/tauri.${platform}.conf.json`));
    assert.equal(platformConfig.app.windows[0].title, config.productName);
    assert.equal(platformConfig.plugins?.updater, undefined);
  }
});
