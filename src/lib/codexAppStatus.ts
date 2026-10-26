import type { CodexProcessInfo } from "../types/index.ts";

export function codexAppStatus(info: CodexProcessInfo) {
  const count = info.desktop_count;
  return {
    label: count == null
      ? "Codex App count unavailable"
      : `${count} Codex App${count === 1 ? "" : "s"} running`,
    backgroundRunning: count === 0 && info.count > 0,
  };
}
