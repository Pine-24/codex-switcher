import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

const launcher = readFileSync(new URL("../scripts/Start-PineForWsl.ps1", import.meta.url), "utf8");
const probe = launcher.match(/\$probe = @'\r?\n([\s\S]*?)\r?\n'@/)?.[1];
assert.ok(probe);

function fixture(context: { after: (cleanup: () => void) => void }) {
  const directory = mkdtempSync(join(tmpdir(), "pine-process-probe-"));
  context.after(() => rmSync(directory, { recursive: true, force: true }));
  mkdirSync(join(directory, "self"));
  writeFileSync(join(directory, "self", "comm"), "sh\n");
  const process = (pid: number, name: string, parent: number | string) => {
    mkdirSync(join(directory, String(pid)), { recursive: true });
    writeFileSync(join(directory, String(pid), "comm"), `${name}\n`);
    writeFileSync(join(directory, String(pid), "status"), `PPid:\t${parent}\n`);
    writeFileSync(join(directory, String(pid), "cmdline"), "PRIVATE-ARGUMENT-NOT-FOR-OUTPUT");
  };
  process(1, "systemd", 0);
  return {
    process,
    run: () => spawnSync("/bin/sh", ["-c", probe!.replaceAll("/proc", directory)], { encoding: "utf8" }),
  };
}

test("WSL probe allows an inspected process list without Codex", { skip: process.platform === "win32" }, (context) => {
  const setup = fixture(context);
  setup.process(10, "bash", 1);
  const result = setup.run();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, "");
});

test("WSL probe reports Desktop, SSH, and orphan blockers without command lines", { skip: process.platform === "win32" }, (context) => {
  const setup = fixture(context);
  setup.process(10, "ChatGPT", 1);
  setup.process(11, "sshd", 1);
  setup.process(20, "codex", 10);
  setup.process(21, "codex", 11);
  setup.process(22, "codex", 1);
  setup.process(23, "codex-code-mode", 22);
  const result = setup.run();
  assert.equal(result.status, 3, result.stderr);
  assert.deepEqual(result.stdout.trim().split("\n").sort(), [
    "20 codex 10 desktop", "21 codex 11 ssh", "22 codex 1 orphan", "23 codex-code-mode 22 codex",
  ]);
  assert.ok(!result.stdout.includes("PRIVATE"));
});

test("WSL probe refuses an unreadable parent identity", { skip: process.platform === "win32" }, (context) => {
  const setup = fixture(context);
  setup.process(20, "codex", 999);
  assert.equal(setup.run().status, 2);
});

test("WSL probe bounds output and fails closed on too many blockers", { skip: process.platform === "win32" }, (context) => {
  const setup = fixture(context);
  for (let pid = 100; pid < 133; pid++) setup.process(pid, "codex", 1);
  const result = setup.run();
  assert.equal(result.status, 2);
  assert.equal(result.stdout.trim().split("\n").length, 32);
});
