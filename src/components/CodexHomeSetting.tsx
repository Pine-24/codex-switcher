import { useEffect, useState } from "react";
import { invokeBackend } from "../lib/platform";

interface CodexHomeSettings {
  path: string;
  override_path: string | null;
}

export function CodexHomeSetting({ onSaved }: { onSaved: () => void }) {
  const [settings, setSettings] = useState<CodexHomeSettings | null>(null);
  const [path, setPath] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let disposed = false;
    void invokeBackend<CodexHomeSettings>("get_codex_home_settings").then((value) => {
      if (disposed) return;
      setSettings(value);
      setPath(value.override_path ?? "");
    }).catch((reason) => {
      if (!disposed) setError(String(reason));
    });
    return () => { disposed = true; };
  }, []);

  const save = async () => {
    setSaving(true);
    setError(null);
    try {
      const value = await invokeBackend<CodexHomeSettings>("set_codex_home_override", { path: path || null });
      setSettings(value);
      setPath(value.override_path ?? "");
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
        Choose the folder Codex actually uses for auth.json. Windows and Ubuntu use their own user folders by default. For Windows Codex connected to WSL, select its WSL folder, such as {String.raw`\\wsl.localhost\Ubuntu\home\your-user\.codex`}. Leave blank to use CODEX_HOME or the default folder on this device.
      </p>
      <input id="codex-home" value={path} onChange={(event) => setPath(event.target.value)} disabled={saving} spellCheck={false} placeholder="Default folder" className="w-full rounded-lg border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-800 px-3 py-2 text-sm text-gray-900 dark:text-gray-100" />
      <div className="flex gap-2">
        <button onClick={() => void browse()} disabled={saving} className="px-3 py-2 text-sm rounded-lg bg-gray-100 dark:bg-gray-800">Choose folder...</button>
        <button onClick={() => void save()} disabled={saving || (!!settings && path === (settings.override_path ?? ""))} className="px-3 py-2 text-sm rounded-lg bg-gray-100 dark:bg-gray-800 disabled:opacity-50">{saving ? "Saving..." : "Save folder"}</button>
      </div>
      {settings && <p className="text-xs text-gray-500 dark:text-gray-400 break-all">Current folder: {settings.path}</p>}
      <p className="text-xs text-gray-500 dark:text-gray-400">Saving only changes this device's destination; credentials are not copied. Then switch to the desired account. Switcher asks before closing running Codex sessions, including WSL sessions. WSL shutdown requires Python 3 with pidfd support in the selected distribution.</p>
      {error && <p role="alert" className="text-sm text-red-600 dark:text-red-300">{error}</p>}
    </div>
  );
}
