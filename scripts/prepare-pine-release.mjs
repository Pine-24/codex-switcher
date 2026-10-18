import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const VERSION = "0.2.21-pine.1";
export const TAG = `v${VERSION}`;
export const SOURCE_BASE = "e69e4260e22d2b8336d682e432e6795ccefa2368";
const OLD_VERSION = "0.2.20";

function replaceExactlyOnce(text, pattern, replacement, label) {
  assert.equal([...text.matchAll(new RegExp(pattern.source, `${pattern.flags}g`))].length, 1,
    `Expected one ${label}; refusing to rewrite an unexpected source version`);
  return text.replace(pattern, replacement);
}

// Release-only metadata. Never reads credentials, changes dependencies or edits main.
// Validate all inputs before writing any of the four prepared files.
export function prepareRelease(root) {
  const read = (name) => fs.readFileSync(path.join(root, name), "utf8");
  const pkg = JSON.parse(read("package.json"));
  const config = JSON.parse(read("src-tauri/tauri.conf.json"));
  assert.equal(pkg.name, "codex-switcher");
  assert.equal(pkg.version, OLD_VERSION);
  assert.equal(config.version, OLD_VERSION);
  const manifest = replaceExactlyOnce(read("src-tauri/Cargo.toml"),
    /(^\[package\][\s\S]*?^version\s*=\s*")0\.2\.20(")/m,
    `$1${VERSION}$2`, "Cargo package version");
  const lock = replaceExactlyOnce(read("src-tauri/Cargo.lock"),
    /(\[\[package\]\]\r?\nname = "codex-switcher"\r?\nversion = ")0\.2\.20(")/,
    `$1${VERSION}$2`, "Cargo.lock root package");
  pkg.version = VERSION;
  config.version = VERSION;
  config.productName = "Codex Switcher Pine";
  config.identifier = "com.pine24.codex-switcher";
  for (const window of config.app?.windows ?? []) window.title = "Codex Switcher Pine";
  config.bundle = {
    ...config.bundle,
    publisher: "Pine-24",
    targets: ["nsis"],
    createUpdaterArtifacts: false,
    windows: {
      ...config.bundle?.windows,
      nsis: { ...config.bundle?.windows?.nsis, installMode: "currentUser" },
    },
  };
  // No unsigned in-app updates, no upstream update feed and no borrowed signing key.
  config.plugins = { ...config.plugins, updater: { pubkey: "", endpoints: [] } };
  const changes = new Map([
    ["package.json", `${JSON.stringify(pkg, null, 2)}\n`],
    ["src-tauri/tauri.conf.json", `${JSON.stringify(config, null, 2)}\n`],
    ["src-tauri/Cargo.toml", manifest],
    ["src-tauri/Cargo.lock", lock],
  ]);
  for (const [name, content] of changes) fs.writeFileSync(path.join(root, name), content, "utf8");
  return [...changes.keys()];
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  prepareRelease(process.cwd());
  console.log(`Prepared ${TAG}: unsigned Windows x64 NSIS, manual updates only`);
}
