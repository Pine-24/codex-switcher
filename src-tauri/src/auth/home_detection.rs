use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HomeCandidate {
    pub path: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct HomeSelection {
    pub path: Option<String>,
    pub source: String,
    pub candidates: Vec<HomeCandidate>,
    pub error: Option<String>,
}

#[derive(Default)]
pub(super) struct Discovery {
    pub live: Vec<HomeCandidate>,
    pub available: Vec<HomeCandidate>,
    pub unresolved: Option<String>,
}

static SELECTIONS: Mutex<Vec<(PathBuf, HomeSelection, std::time::Instant)>> =
    Mutex::new(Vec::new());

fn selected(candidate: HomeCandidate) -> HomeSelection {
    HomeSelection {
        path: Some(candidate.path.clone()),
        source: candidate.source.clone(),
        candidates: vec![candidate],
        error: None,
    }
}

fn deduplicate(candidates: Vec<HomeCandidate>) -> Vec<HomeCandidate> {
    let mut result: Vec<HomeCandidate> = Vec::new();
    for candidate in candidates {
        if !result.iter().any(|other| other.path == candidate.path) {
            result.push(candidate);
        }
    }
    result
}

fn choose(discovery: Discovery, remembered: Option<HomeCandidate>, default: &str) -> HomeSelection {
    let live = deduplicate(discovery.live);
    let candidates = if live.is_empty() {
        if let Some(mut previous) = remembered {
            previous.source = "Previously detected on this device".into();
            return selected(previous);
        }
        if let Some(error) = discovery.unresolved {
            return HomeSelection {
                path: None,
                source: "Selection required".into(),
                candidates: deduplicate(discovery.available),
                error: Some(error),
            };
        }
        deduplicate(discovery.available)
    } else {
        live
    };
    match candidates.as_slice() {
        [] => selected(HomeCandidate { path: default.into(), source: "OS default".into() }),
        [candidate] => selected(candidate.clone()),
        _ => HomeSelection {
            path: None,
            source: "Selection required".into(),
            candidates,
            error: Some("Multiple Codex account folders were found. Select the Codex environment you want to switch in Settings.".into()),
        },
    }
}

pub fn clear_cache() -> Result<()> {
    let key = super::get_config_dir()?;
    SELECTIONS
        .lock()
        .map_err(|_| anyhow::anyhow!("Account-folder selection lock failed"))?
        .retain(|(path, _, _)| *path != key);
    Ok(())
}

pub fn refresh_for_operation(force: bool) -> Result<()> {
    let key = super::get_config_dir()?;
    let mut selections = SELECTIONS
        .lock()
        .map_err(|_| anyhow::anyhow!("Account-folder selection lock failed"))?;
    selections.retain(|(path, _, at)| {
        *path != key || (!force && at.elapsed() < std::time::Duration::from_secs(10))
    });
    drop(selections);
    let selection = resolve()?;
    anyhow::ensure!(
        selection.path.is_some(),
        "Codex account folder needs attention: {}",
        selection
            .error
            .unwrap_or_else(|| "Select a Codex account folder in Settings".into())
    );
    Ok(())
}

fn default_home() -> Result<PathBuf> {
    #[cfg(test)]
    {
        Ok(super::test_support::home_dir()?.join(".codex"))
    }
    #[cfg(not(test))]
    {
        Ok(dirs::home_dir()
            .context("Could not find home directory")?
            .join(".codex"))
    }
}

pub fn resolve() -> Result<HomeSelection> {
    if let Some(path) = super::load_app_settings()?.codex_home_override {
        return explicit(path, "Manually selected");
    }
    #[cfg(not(test))]
    if let Some(path) = std::env::var_os("CODEX_HOME") {
        return explicit(
            path.into_string()
                .map_err(|_| anyhow::anyhow!("CODEX_HOME is not valid Unicode"))?,
            "CODEX_HOME",
        );
    }
    let key = super::get_config_dir()?;
    let mut selections = SELECTIONS
        .lock()
        .map_err(|_| anyhow::anyhow!("Account-folder selection lock failed"))?;
    if let Some((_, selection, _)) = selections.iter().find(|(path, _, _)| *path == key) {
        return Ok(selection.clone());
    }
    let default = default_home()?;
    let remembered_file = key.join("detected-codex-home.json");
    let selection = (|| -> Result<HomeSelection> {
        let remembered = match std::fs::read(&remembered_file) {
            Ok(bytes) => Some(serde_json::from_slice::<HomeCandidate>(&bytes).context("Invalid remembered Codex folder; select a folder manually")?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("Could not read the remembered Codex folder"),
        };
        let discovery = discover(&default)?;
        let selection = choose(discovery, remembered, &default.to_string_lossy());
        if let Some(path) = &selection.path {
            let path = super::validate_codex_home(path)?;
            if selection.source == "Previously detected on this device" {
                anyhow::ensure!(path.is_dir(), "The previously detected Codex folder is unavailable. Select a folder in Settings; no fallback was used.");
            }
            if selection.source != "OS default" {
                std::fs::create_dir_all(&key)?;
                std::fs::write(&remembered_file, serde_json::to_vec(&HomeCandidate {
                    path: path.to_string_lossy().into_owned(),
                    source: selection.source.clone(),
                })?)?;
            }
        }
        Ok(selection)
    })().unwrap_or_else(|error| HomeSelection {
        path: None,
        source: "Detection unavailable".into(),
        candidates: Vec::new(),
        error: Some(format!("Could not determine the Codex account folder: {error}. Select it manually in Settings or retry detection.")),
    });
    selections.push((key, selection.clone(), std::time::Instant::now()));
    Ok(selection)
}

fn explicit(path: String, source: &str) -> Result<HomeSelection> {
    let path = super::validate_codex_home(&path)?;
    Ok(selected(HomeCandidate {
        path: path.to_string_lossy().into_owned(),
        source: source.into(),
    }))
}

fn discover(default: &std::path::Path) -> Result<Discovery> {
    let mut discovery = Discovery::default();
    if default.join("auth.json").try_exists()? {
        discovery.available.push(HomeCandidate {
            path: default.to_string_lossy().into_owned(),
            source: "Existing native login".into(),
        });
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        use crate::commands::process::linux_process;
        for process in linux_process::snapshot().map_err(anyhow::Error::msg)? {
            if !linux_process::is_codex_executable(&process.executable) {
                continue;
            }
            let bytes = match std::fs::read(format!("/proc/{}/environ", process.pid)) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => anyhow::bail!("Cannot inspect a running Codex process"),
            };
            let path = linux_environment_home(&bytes, default)?;
            discovery.live.push(HomeCandidate {
                path: path.to_string_lossy().into_owned(),
                source: "Running Codex (Ubuntu/Linux)".into(),
            });
        }
    }
    #[cfg(all(windows, not(test)))]
    super::home_probe::discover_wsl(&mut discovery, default)?;
    Ok(discovery)
}

#[cfg(any(target_os = "linux", test))]
fn linux_environment_home(bytes: &[u8], default: &std::path::Path) -> Result<PathBuf> {
    let get = |name: &[u8]| -> Result<Option<String>> {
        bytes
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(name))
            .map(|value| {
                String::from_utf8(value.to_vec()).context("Codex home is not valid Unicode")
            })
            .transpose()
    };
    if let Some(path) = get(b"CODEX_HOME=")? {
        return super::validate_codex_home(&path);
    }
    if let Some(home) = get(b"HOME=")? {
        return Ok(super::validate_codex_home(&home)?.join(".codex"));
    }
    Ok(default.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate(path: &str) -> HomeCandidate {
        HomeCandidate {
            path: path.into(),
            source: "Observed".into(),
        }
    }

    #[test]
    fn live_wsl_wins_over_an_unrelated_native_auth_file() {
        let result = choose(
            Discovery {
                live: vec![candidate("/wsl")],
                available: vec![candidate("/native")],
                ..Default::default()
            },
            None,
            "/native",
        );
        assert_eq!(result.path.as_deref(), Some("/wsl"));
    }

    #[test]
    fn duplicate_processes_for_one_home_are_not_ambiguous() {
        assert_eq!(
            choose(
                Discovery {
                    live: vec![candidate("/same"), candidate("/same")],
                    ..Default::default()
                },
                None,
                "/native"
            )
            .path
            .as_deref(),
            Some("/same")
        );
    }

    #[test]
    fn conflicting_live_homes_require_selection_even_with_a_remembered_target() {
        let result = choose(
            Discovery {
                live: vec![candidate("/a"), candidate("/b")],
                ..Default::default()
            },
            Some(candidate("/a")),
            "/native",
        );
        assert!(result.path.is_none());
        assert_eq!(result.candidates.len(), 2);
    }

    #[test]
    fn stopping_codex_does_not_change_the_remembered_destination() {
        assert_eq!(
            choose(
                Discovery {
                    available: vec![candidate("/native")],
                    ..Default::default()
                },
                Some(candidate("/wsl")),
                "/native"
            )
            .path
            .as_deref(),
            Some("/wsl")
        );
    }

    #[test]
    fn multiple_existing_logins_without_live_evidence_are_not_guessed() {
        assert!(choose(
            Discovery {
                available: vec![candidate("/native"), candidate("/wsl")],
                ..Default::default()
            },
            None,
            "/native"
        )
        .path
        .is_none());
        assert_eq!(
            choose(Discovery::default(), None, "/native")
                .path
                .as_deref(),
            Some("/native")
        );
    }

    #[test]
    fn unobserved_stopped_distributions_do_not_silently_choose_native() {
        let result = choose(
            Discovery {
                available: vec![candidate("/native")],
                unresolved: Some("Start Codex to detect its folder".into()),
                ..Default::default()
            },
            None,
            "/native",
        );
        assert!(result.path.is_none());
        assert_eq!(result.candidates.len(), 1);
    }

    #[test]
    fn unavailable_remembered_directory_blocks_instead_of_falling_back() {
        let _home = super::super::test_support::TestHome::new();
        let config = super::super::get_config_dir().unwrap();
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("detected-codex-home.json"),
            serde_json::to_vec(&candidate(config.join("missing").to_str().unwrap())).unwrap(),
        )
        .unwrap();
        assert!(resolve().unwrap().path.is_none());
        assert!(super::super::get_codex_home().is_err());
        assert!(!default_home().unwrap().join("auth.json").exists());
    }

    #[test]
    fn manual_override_bypasses_broken_detection_state() {
        let _home = super::super::test_support::TestHome::new();
        let config = super::super::get_config_dir().unwrap();
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(config.join("detected-codex-home.json"), "invalid").unwrap();
        let mut settings = super::super::load_app_settings().unwrap();
        settings.codex_home_override = Some(config.to_str().unwrap().into());
        super::super::save_app_settings(&settings).unwrap();
        assert_eq!(resolve().unwrap().source, "Manually selected");
    }

    #[cfg(unix)]
    #[test]
    fn process_environment_uses_only_home_fields_and_rejects_relative_paths() {
        assert_eq!(
            linux_environment_home(
                b"SECRET=not-returned\0HOME=/users/test\0CODEX_HOME=/custom\0",
                std::path::Path::new("/default")
            )
            .unwrap(),
            PathBuf::from("/custom")
        );
        assert!(
            linux_environment_home(b"CODEX_HOME=relative\0", std::path::Path::new("/default"))
                .is_err()
        );
    }

    #[test]
    fn resolver_never_reads_or_copies_authentication_contents() {
        let _home = super::super::test_support::TestHome::new();
        let default = default_home().unwrap();
        std::fs::create_dir_all(&default).unwrap();
        std::fs::write(default.join("auth.json"), "not-json-secret-fixture").unwrap();
        let selection = resolve().unwrap();
        assert_eq!(selection.path.as_deref(), default.to_str());
        assert_eq!(
            std::fs::read_to_string(default.join("auth.json")).unwrap(),
            "not-json-secret-fixture"
        );
        clear_cache().unwrap();
        assert_eq!(
            resolve().unwrap().source,
            "Previously detected on this device"
        );
    }
}
