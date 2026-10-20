import { useEffect, useState } from "react";
import { invokeBackend } from "../lib/platform";

interface CodexHomeSettings {
  path: string | null;
  override_path: string | null;
  source: string;
  candidates: { path: string; source: string }[];
  error: string | null;
}

export function CodexHomeSetting({ onSaved }: { onSaved: () => void }) {
  const [settings, setSettings] = useState<CodexHomeSettings | null>(null);
  const [path, setPath] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [advanced, setAdvanced] = useState(false);

  useEffect(() => {
    let disposed = false;
    void invokeBackend<CodexHomeSettings>("get_codex_home_settings").then((value) => {
      if (disposed) return;
      setSettings(value);
      setPath(value.override_path ?? "");
      setAdvanced(!!value.override_path);
    }).catch((reason) => {
      if (!disposed) setError(String(reason));
    });
    return () => { disposed = true; };
  }, []);

  const save = async (selectedPath = path) => {
    setSaving(true);
    setError(null);
    try {
      const value = await invokeBackend<CodexHomeSettings>("set_codex_home_override", { path: selectedPath || null });
      setSettings(value);
      setPath(value.override_path ?? "");
      onSaved();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  const detect = async () => {
    setSaving(true);
    setError(null);
    try {
      const value = await invokeBackend<CodexHomeSettings>("refresh_codex_home_detection");
      setSettings(value);
      setPath(value.override_path ?? "");
      setAdvanced(false);
      onSaved();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };

  const browse = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false, title: "Choose the folder containing Codex auth.json" });
      if (selected) setPath(selected);
    } catch (reason) {
      setError(String(reason));
    }
  };

  return (
    <div className="space-y-2">
      <label htmlFor="codex-home" className="block text-sm font-medium text-gray-900 dark:text-gray-100">Codex account folder</label>
      <p className="text-xs text-gray-500 dark:text-gray-400">
        Switcher automatically detects the account folder. Running Codex provides the strongest evidence; the detected folder is remembered after Codex closes. If multiple environments are found, choose the one you want to switch.
      </p>
      {settings?.path && <p className="text-xs text-gray-500 dark:text-gray-400 break-all">{settings.source}: {settings.path}</p>}
      {settings?.error && <p role="alert" className="text-sm text-red-600 dark:text-red-300">{settings.error}</p>}
      {!settings?.path && settings?.candidates.map((candidate) => (
        <button key={candidate.path} onClick={() => void save(candidate.path)} disabled={saving} className="block w-full rounded-lg bg-gray-100 dark:bg-gray-800 p-2 text-left text-xs break-all">
          Use {candidate.source}: {candidate.path}
        </button>
      ))}
      <button onClick={() => void detect()} disabled={saving} className="px-3 py-2 text-sm rounded-lg bg-gray-100 dark:bg-gray-800 disabled:opacity-50">{saving ? "Checking..." : settings?.override_path ? "Use automatic selection" : "Detect again"}</button>
      <button onClick={() => setAdvanced(!advanced)} disabled={saving} className="block text-xs text-gray-500 underline">{advanced ? "Hide manual settings" : "Advanced: choose a folder manually"}</button>
      {advanced && <>
      <p className="text-xs text-gray-500 dark:text-gray-400">Manual selection overrides detection. For WSL, use a path such as {String.raw`\\wsl.localhost\Ubuntu\home\your-user\.codex`}. CODEX_HOME, if set for Switcher, also takes precedence over automatic detection.</p>
      <input id="codex-home" value={path} onChange={(event) => setPath(event.target.value)} disabled={saving} spellCheck={false} placeholder="Default folder" className="w-full rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-gray-100" />
      <div className="flex gap-2">
        <button onClick={() => void browse()} disabled={saving} className="px-3 py-2 text-sm rounded-lg bg-gray-100 dark:bg-gray-800">Choose folder...</button>
        <button onClick={() => void save()} disabled={saving || (!!settings && path === (settings.override_path ?? ""))} className="px-3 py-2 text-sm rounded-lg bg-gray-100 dark:bg-gray-800 disabled:opacity-50">{saving ? "Saving..." : "Save folder"}</button>
      </div>
      </>}
      <p className="text-xs text-gray-500 dark:text-gray-400">Detection does not copy credentials or stop Codex. Switcher asks before closing sessions. WSL process inspection requires Python 3; shutdown also requires pidfd support.</p>
      {error && <p role="alert" className="text-sm text-red-600 dark:text-red-300">{error}</p>}
    </div>
  );
}
