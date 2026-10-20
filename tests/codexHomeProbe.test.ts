import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

const source = readFileSync(new URL("../src-tauri/src/auth/home_probe.rs", import.meta.url), "utf8");
const probe = source.match(/const WSL_HOME_PROBE: &str = r#"([\s\S]*?)"#;/)?.[1];
assert.ok(probe);

test("WSL discovery distinguishes existing login, live Codex, and Switcher without reading credentials", { skip: process.platform === "win32" }, (context) => {
  const root = mkdtempSync(join(tmpdir(), "switcher-home-probe-"));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const proc = join(root, "proc");
  const home = join(root, "user with spaces");
  mkdirSync(join(proc, "self"), { recursive: true });
  mkdirSync(join(proc, "10"));
  mkdirSync(join(home, ".codex"), { recursive: true });
  writeFileSync(join(proc, "self", "comm"), "sh\n");
  writeFileSync(join(proc, "10", "comm"), "codex-switcher\n");
  const run = () => spawnSync("/bin/sh", ["-c", probe!.replaceAll("/proc", proc)], { env: { ...process.env, HOME: home }, encoding: "utf8" });
  assert.equal(run().stdout, `${home}/.codex\0${0}\0${0}\0`);
  writeFileSync(join(home, ".codex", "auth.json"), "SECRET-NOT-FOR-OUTPUT");
  assert.equal(run().stdout, `${home}/.codex\0${1}\0${0}\0`);
  writeFileSync(join(proc, "10", "comm"), "codex\n");
  const result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, `${home}/.codex\0${1}\0${1}\0`);
});
