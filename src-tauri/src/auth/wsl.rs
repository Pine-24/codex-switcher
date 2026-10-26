use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct ProcessRecord {
    pid: u32,
}

#[derive(Deserialize)]
struct HelperResult {
    status: String,
    #[serde(rename = "desktopCount")]
    desktop_count: Option<usize>,
    #[serde(rename = "blockingProcesses")]
    blocking_processes: Vec<ProcessRecord>,
    #[serde(rename = "remainingTargetPids")]
    remaining_target_pids: Option<Vec<u32>>,
    #[serde(default)]
    targets: Vec<ProcessRecord>,
}

pub fn blocking_count() -> Result<usize> {
    Ok(run(false)?.map_or(0, |result| result.blocking_processes.len()))
}

pub fn status_counts() -> Result<(usize, Option<usize>)> {
    Ok(run(false)?.map_or((0, Some(0)), |result| {
        (result.blocking_processes.len(), result.desktop_count)
    }))
}

pub fn close_selected() -> Result<usize> {
    Ok(run(true)?.map_or(0, |result| result.targets.len()))
}

fn parse_result(output: &str, stopping: bool) -> Result<HelperResult> {
    let mut lines = output.lines().filter(|line| !line.trim().is_empty());
    let first = lines.next().context("WSL helper returned no result")?;
    if stopping {
        #[derive(Deserialize)]
        struct Terminating {
            status: String,
            targets: Vec<ProcessRecord>,
        }
        let starting: Terminating =
            serde_json::from_str(first).context("Invalid WSL shutdown response")?;
        anyhow::ensure!(
            starting.status == "terminating",
            "WSL shutdown was not started"
        );
        let mut result: HelperResult =
            serde_json::from_str(lines.next().context("Missing WSL shutdown verification")?)?;
        anyhow::ensure!(
            lines.next().is_none()
                && result.status == "stopped"
                && result.blocking_processes.is_empty()
                && result
                    .remaining_target_pids
                    .as_ref()
                    .is_some_and(Vec::is_empty),
            "WSL Codex processes remain; switching was cancelled"
        );
        result.targets = starting.targets;
        Ok(result)
    } else {
        let result: HelperResult =
            serde_json::from_str(first).context("Invalid WSL process response")?;
        anyhow::ensure!(
            lines.next().is_none()
                && result.status == "preview"
                && result
                    .blocking_processes
                    .iter()
                    .all(|process| process.pid > 0),
            "Invalid WSL process snapshot"
        );
        Ok(result)
    }
}

fn run(stopping: bool) -> Result<Option<HelperResult>> {
    #[cfg(windows)]
    {
        use std::io::{Read, Write};
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let home = super::get_codex_home()?;
        let home = home.to_string_lossy();
        let Some(distribution) = super::switcher::wsl_distribution(&home) else {
            return Ok(None);
        };
        let linux_home = format!(
            "/{}",
            home.trim_end_matches('\\')
                .split('\\')
                .skip(4)
                .collect::<Vec<_>>()
                .join("/")
        );
        let mut arguments = vec!["stop_wsl_codex.py", "--home", &linux_home];
        if stopping {
            arguments.push("--stop");
        }
        let source = format!(
            "import sys\nsys.argv = {}\n{}",
            serde_json::to_string(&arguments)?,
            include_str!("../../../scripts/stop_wsl_codex.py")
        );
        let mut child = Command::new("wsl.exe")
            .creation_flags(0x08000000)
            .args(["--distribution", distribution, "--exec", "/usr/bin/python3", "-"])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().context("Could not run WSL process helper; Ubuntu needs /usr/bin/python3 with pidfd support")?;
        let stdout = child.stdout.take().context("Missing WSL stdout")?;
        let reader = std::thread::spawn(move || {
            let mut output = String::new();
            stdout
                .take(1024 * 1024)
                .read_to_string(&mut output)
                .map(|_| output)
        });
        let mut stdin = child.stdin.take().context("Missing WSL stdin")?;
        let writer = std::thread::spawn(move || stdin.write_all(source.as_bytes()));
        let deadline = Instant::now() + Duration::from_secs(if stopping { 30 } else { 10 });
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                anyhow::bail!("WSL process check timed out; switching was cancelled");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        anyhow::ensure!(status.success(), "WSL helper could not verify or stop Codex. Close remaining WSL sessions manually; Python 3 with pidfd support is required. Switching was cancelled.");
        writer
            .join()
            .map_err(|_| anyhow::anyhow!("WSL input writer failed"))??;
        let output = reader
            .join()
            .map_err(|_| anyhow::anyhow!("WSL output reader failed"))??;
        return parse_result(&output, stopping).map(Some);
    }
    #[cfg(not(windows))]
    {
        let _ = stopping;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_unverified_results_fail_closed() {
        for output in [
            "",
            "{}",
            r#"{"status":"preview"}"#,
            r#"{"status":"failed","blockingProcesses":[]}"#,
        ] {
            assert!(parse_result(output, false).is_err());
        }
        assert!(parse_result("{\"status\":\"terminating\",\"targets\":[]}\n{\"status\":\"blocked\",\"blockingProcesses\":[]}", true).is_err());
    }

    #[test]
    fn preview_includes_remote_pids_without_merging_native_ids() {
        assert_eq!(
            parse_result(
                r#"{"status":"preview","blockingProcesses":[{"pid":42}]}"#,
                false
            )
            .unwrap()
            .blocking_processes[0]
                .pid,
            42
        );
    }

    #[test]
    fn preview_keeps_desktop_count_separate_and_missing_counts_unknown() {
        let preview = parse_result(
            r#"{"status":"preview","desktopCount":1,"blockingProcesses":[{"pid":42},{"pid":43}]}"#,
            false,
        )
        .unwrap();
        assert_eq!(preview.desktop_count, Some(1));
        assert_eq!(preview.blocking_processes.len(), 2);
        let missing =
            parse_result(r#"{"status":"preview","blockingProcesses":[]}"#, false).unwrap();
        assert_eq!(missing.desktop_count, None);
    }

    #[test]
    fn shutdown_requires_no_remaining_processes() {
        let start = "{\"status\":\"terminating\",\"targets\":[{\"pid\":42}]}\n";
        assert!(parse_result(
            &(start.to_owned()
                + r#"{"status":"stopped","blockingProcesses":[],"remainingTargetPids":[42]}"#),
            true
        )
        .is_err());
        assert_eq!(
            parse_result(
                &(start.to_owned()
                    + r#"{"status":"stopped","blockingProcesses":[],"remainingTargetPids":[]}"#),
                true
            )
            .unwrap()
            .targets
            .len(),
            1
        );
    }
}
