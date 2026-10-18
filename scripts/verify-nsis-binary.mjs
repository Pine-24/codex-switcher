import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const UNBUNDLED = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_UNK", "ascii");
const NSIS = Buffer.from("__TAURI_BUNDLE_TYPE_VAR_NSS", "ascii");
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

// Tauri CLI 2.10.1 patches the FIRST bundle marker before NSIS compression and
// restores the original build output afterwards. Compare against precisely
// that expected patch, not an arbitrary normalized or partially compared file.
// Source: tauri-apps/tauri, tauri-cli-v2.10.1,
// crates/tauri-bundler/src/bundle.rs (patch_binary and bundle_project).
export function verifyNsisBinary(built, installed) {
  assert.ok(Buffer.isBuffer(built) && Buffer.isBuffer(installed), "Expected binary buffers");
  assert.equal(installed.length, built.length, "Installed executable length differs from build");
  const offset = built.indexOf(UNBUNDLED);
  assert.ok(offset >= 0, "Build output is missing the expected Tauri bundle marker");
  const expected = Buffer.from(built);
  NSIS.copy(expected, offset);
  assert.ok(installed.equals(expected),
    `Installed executable differs beyond the Tauri NSIS marker: expected ${sha256(expected)}, got ${sha256(installed)}`);
  return {
    bundle_marker_offset: offset,
    build_sha256: sha256(built),
    expected_nsis_sha256: sha256(expected),
    installed_sha256: sha256(installed),
  };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [builtPath, installedPath, reportPath] = process.argv.slice(2);
  assert.ok(builtPath && installedPath && reportPath,
    "Usage: node verify-nsis-binary.mjs BUILD_EXE INSTALLED_EXE REPORT_JSON");
  const report = verifyNsisBinary(fs.readFileSync(builtPath), fs.readFileSync(installedPath));
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  console.log("Installed executable matches the exact Tauri NSIS bundle-marker patch");
}
