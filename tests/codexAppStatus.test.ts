import assert from "node:assert/strict";
import test from "node:test";
import { codexAppStatus } from "../src/lib/codexAppStatus.ts";

const blocking = { count: 4, wsl_count: 3, background_count: 0, can_switch: false, pids: [10] };

test("one desktop with WSL backends is displayed as one app", () => {
  assert.deepEqual(codexAppStatus({ ...blocking, desktop_count: 1 }), {
    label: "1 Codex App running", backgroundRunning: false,
  });
});

test("multiple desktops are counted independently of their backends", () => {
  assert.equal(codexAppStatus({ ...blocking, desktop_count: 2 }).label, "2 Codex Apps running");
});

test("headless activity remains visible when no desktop apps are running", () => {
  assert.deepEqual(codexAppStatus({ ...blocking, desktop_count: 0 }), {
    label: "0 Codex Apps running", backgroundRunning: true,
  });
  assert.equal(codexAppStatus({ ...blocking, desktop_count: 0, count: 0, can_switch: true }).backgroundRunning, false);
});

test("an unknown app count never falls back to the process count or zero", () => {
  for (const desktop_count of [undefined, null]) {
    assert.equal(codexAppStatus({ ...blocking, desktop_count }).label, "Codex App count unavailable");
  }
});
