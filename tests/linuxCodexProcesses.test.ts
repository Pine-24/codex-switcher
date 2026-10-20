import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

test("Linux account switching detects headless Codex and verified desktop roots", { skip: process.platform !== "linux" }, (context) => {
  const directory = mkdtempSync(join(tmpdir(), "switcher-linux-process-tests-"));
  context.after(() => rmSync(directory, { recursive: true, force: true }));
  const source = fileURLToPath(new URL("../src-tauri/src/commands/linux_process.rs", import.meta.url));
  const wrapper = join(directory, "tests.rs");
  const executable = join(directory, "tests");
  writeFileSync(wrapper, `#[path = ${JSON.stringify(source)}] mod linux_process;`);
  const compiled = spawnSync(process.env.RUSTC ?? "rustc", ["--edition=2021", "--test", wrapper, "-o", executable], { encoding: "utf8" });
  assert.ifError(compiled.error);
  assert.equal(compiled.status, 0, compiled.stderr);
  const result = spawnSync(executable, [], { encoding: "utf8" });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stdout + result.stderr);
});
