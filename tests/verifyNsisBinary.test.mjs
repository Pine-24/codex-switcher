import assert from "node:assert/strict";
import test from "node:test";
import { verifyNsisBinary } from "../scripts/verify-nsis-binary.mjs";

const marker = "__TAURI_BUNDLE_TYPE_VAR_";
const built = () => Buffer.from(`binary-prefix\x00${marker}UNK\x00binary-suffix`);
const installed = () => Buffer.from(`binary-prefix\x00${marker}NSS\x00binary-suffix`);

test("verifies the exact NSIS marker substitution and reports whole-file hashes", () => {
  const report = verifyNsisBinary(built(), installed());
  assert.equal(report.expected_nsis_sha256, report.installed_sha256);
  assert.notEqual(report.build_sha256, report.installed_sha256);
  assert.equal(report.bundle_marker_offset, 14);
});

test("a changed byte outside the marker is rejected", () => {
  const bytes = installed();
  bytes[0] ^= 1;
  assert.throws(() => verifyNsisBinary(built(), bytes), /differs beyond/);
});

test("unpatched and wrong-package executables are rejected", () => {
  assert.throws(() => verifyNsisBinary(built(), built()), /differs beyond/);
  const msi = Buffer.from(installed().toString().replace(`${marker}NSS`, `${marker}MSI`));
  assert.throws(() => verifyNsisBinary(built(), msi), /differs beyond/);
});

test("a missing marker fails instead of falling back to a weak comparison", () => {
  assert.throws(() => verifyNsisBinary(Buffer.from("same"), Buffer.from("same")), /missing/);
});

test("truncated or extended executables are rejected", () => {
  assert.throws(() => verifyNsisBinary(built(), installed().subarray(1)), /length differs/);
  assert.throws(() => verifyNsisBinary(built(), Buffer.concat([installed(), Buffer.of(0)])), /length differs/);
});

test("only the first marker changes, exactly as in the pinned Tauri bundler", () => {
  const extra = Buffer.from(`${marker}UNK`);
  const before = Buffer.concat([built(), extra]);
  const after = Buffer.concat([installed(), extra]);
  assert.doesNotThrow(() => verifyNsisBinary(before, after));
  assert.throws(() => verifyNsisBinary(before, Buffer.concat([installed(), Buffer.from(`${marker}NSS`)])), /differs beyond/);
});
