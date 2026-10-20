use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub(crate) struct LinuxProcess {
    pub pid: u32,
    pub parent: u32,
    pub executable: PathBuf,
}

pub(crate) fn is_codex_executable(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let name = name.strip_suffix(" (deleted)").unwrap_or(name);
    name == "codex" || (name.starts_with("codex-") && !name.starts_with("codex-switcher"))
}

pub(super) fn is_desktop_executable(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some("ChatGPT" | "Codex" | "codex-desktop")
    ) && path
        .parent()
        .is_some_and(|parent| parent.join("resources/codex").is_file())
}

pub(crate) fn snapshot() -> Result<Vec<LinuxProcess>, String> {
    let uid = fs::metadata("/proc/self")
        .map_err(|error| error.to_string())?
        .uid();
    snapshot_at(Path::new("/proc"), uid)
}

fn snapshot_at(root: &Path, uid: u32) -> Result<Vec<LinuxProcess>, String> {
    let mut processes = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        let inspect = || -> std::io::Result<Option<LinuxProcess>> {
            if fs::metadata(entry.path())?.uid() != uid {
                return Ok(None);
            }
            let stat = fs::read_to_string(entry.path().join("stat"))?;
            let fields: Vec<_> = stat
                .rsplit_once(')')
                .map(|(_, tail)| tail.split_whitespace().collect())
                .unwrap_or_default();
            if matches!(fields.first(), Some(&"Z" | &"X")) {
                return Ok(None);
            }
            let parent = fields
                .get(1)
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| std::io::Error::other("Invalid process identity"))?;
            let executable = fs::read_link(entry.path().join("exe"))?;
            Ok(Some(LinuxProcess {
                pid,
                parent,
                executable,
            }))
        };
        match inspect() {
            Ok(Some(process)) => processes.push(process),
            Ok(None) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not inspect Linux process {pid}: {error}")),
        }
    }
    Ok(processes)
}

pub(super) fn desktop_roots(processes: &[LinuxProcess]) -> Vec<&LinuxProcess> {
    processes
        .iter()
        .filter(|process| {
            is_desktop_executable(&process.executable)
                && !processes.iter().any(|parent| {
                    parent.pid == process.parent && parent.executable == process.executable
                })
        })
        .collect()
}

pub(super) fn blocking_pids() -> Result<Vec<u32>, String> {
    let processes = snapshot()?;
    let mut pids: Vec<_> = processes
        .iter()
        .filter(|process| is_codex_executable(&process.executable))
        .map(|process| process.pid)
        .collect();
    pids.extend(desktop_roots(&processes).iter().map(|process| process.pid));
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_includes_sleeping_orphans_and_skips_zombies_and_other_users() {
        use std::os::unix::fs::symlink;
        let root =
            std::env::temp_dir().join(format!("switcher-proc-fixture-{}", std::process::id()));
        for (pid, state) in [(41, "S"), (42, "Z")] {
            let directory = root.join(pid.to_string());
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join("stat"),
                format!("{pid} (codex) {state} 1 0 0"),
            )
            .unwrap();
            symlink("/opt/custom/codex", directory.join("exe")).unwrap();
        }
        let uid = fs::metadata(&root).unwrap().uid();
        let processes = snapshot_at(&root, uid).unwrap();
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].pid, 41);
        assert_eq!(processes[0].parent, 1);
        assert!(snapshot_at(&root, uid + 1).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn includes_headless_helpers_and_excludes_switcher() {
        for path in [
            "/opt/codex/bin/codex",
            "/opt/codex/bin/codex-code-mode-host",
            "/opt/codex/bin/codex (deleted)",
        ] {
            assert!(is_codex_executable(Path::new(path)));
        }
        for path in [
            "/opt/codex-switcher",
            "/opt/codex-switcher-web",
            "/usr/bin/python3",
        ] {
            assert!(!is_codex_executable(Path::new(path)));
        }
    }

    #[test]
    fn inaccessible_proc_is_not_an_empty_success() {
        assert!(snapshot_at(Path::new("/no-such-codex-process-directory"), 0).is_err());
    }

    #[test]
    fn desktop_requires_codex_resources_and_excludes_children() {
        let root =
            std::env::temp_dir().join(format!("switcher-linux-desktop-{}", std::process::id()));
        fs::create_dir_all(root.join("resources")).unwrap();
        let executable = root.join("ChatGPT");
        assert!(!is_desktop_executable(&executable));
        fs::write(root.join("resources/codex"), "fixture").unwrap();
        let processes = vec![
            LinuxProcess {
                pid: 10,
                parent: 1,
                executable: executable.clone(),
            },
            LinuxProcess {
                pid: 11,
                parent: 10,
                executable,
            },
        ];
        assert_eq!(
            desktop_roots(&processes)
                .iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![10]
        );
        fs::remove_dir_all(root).unwrap();
    }
}
