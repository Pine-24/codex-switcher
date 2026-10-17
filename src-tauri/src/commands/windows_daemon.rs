//! Windows managed app-server lifecycle support.
//!
//! A managed daemon can outlive its clients and retain their credentials.
//! Treat it as a switch blocker and stop it only in the confirmed close flow.

use super::WindowsCodexProcess;

pub(super) fn is_managed_daemon(process: &WindowsCodexProcess) -> bool {
    if !process.name.eq_ignore_ascii_case("codex.exe") {
        return false;
    }
    let Some(args) = windows_arguments(&process.command_line) else {
        return false;
    };
    let Some(executable) = args.first() else {
        return false;
    };
    let name = executable.rsplit(['\\', '/']).next().unwrap_or("");
    if !name.eq_ignore_ascii_case("codex.exe") && !name.eq_ignore_ascii_case("codex") {
        return false;
    }
    if args.get(1).map(String::as_str) != Some("app-server") {
        return false;
    }

    let mut managed = false;
    let mut args = args.iter().skip(2);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            // Do not mistake config values, help probes or lifecycle commands
            // for a running server. Matching a substring can target a prompt.
            "--help" | "-h" | "--version" | "-V" | "daemon" => return false,
            "--" => break,
            "-c" | "--config" | "--listen" => {
                if args.next().is_none() {
                    return false;
                }
            }
            "--managed-daemon" => managed = true,
            _ => {}
        }
    }
    managed
}

/// Parse quoting and backslash escaping without executing a shell.
/// Unbalanced quotes are rejected rather than guessed at.
fn windows_arguments(command: &str) -> Option<Vec<String>> {
    let mut input = command.chars().peekable();
    let mut arguments = Vec::new();
    loop {
        while matches!(input.peek(), Some(' ' | '\t')) {
            input.next();
        }
        input.peek()?;
        let mut argument = String::new();
        let mut quoted = false;
        loop {
            if input.peek().is_none() || (!quoted && matches!(input.peek(), Some(' ' | '\t'))) {
                break;
            }
            let mut backslashes = 0;
            while input.peek() == Some(&'\\') {
                input.next();
                backslashes += 1;
            }
            if input.peek() == Some(&'"') {
                argument.extend(std::iter::repeat_n('\\', backslashes / 2));
                input.next();
                if backslashes % 2 == 1 {
                    argument.push('"');
                } else if quoted && input.peek() == Some(&'"') {
                    input.next();
                    argument.push('"');
                } else {
                    quoted = !quoted;
                }
            } else {
                argument.extend(std::iter::repeat_n('\\', backslashes));
                // Backslashes only escape quotes. An unquoted separator
                // following a path's trailing backslash still ends the arg.
                if input.peek().is_none()
                    || (!quoted && matches!(input.peek(), Some(' ' | '\t')))
                {
                    break;
                }
                argument.push(input.next()?);
            }
        }
        if quoted {
            return None;
        }
        arguments.push(argument);
        while matches!(input.peek(), Some(' ' | '\t')) {
            input.next();
        }
        if input.peek().is_none() {
            return Some(arguments);
        }
    }
}

fn parse_daemon_pid(contents: &str) -> Option<u32> {
    let record: serde_json::Value = serde_json::from_str(contents).ok()?;
    let pid = u32::try_from(record.get("pid")?.as_u64()?).ok()?;
    (pid > 0).then_some(pid)
}

#[cfg(windows)]
pub(super) fn stop_for_confirmed_close(process: &WindowsCodexProcess) -> anyhow::Result<()> {
    use anyhow::{bail, Context};
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    if !is_managed_daemon(process) {
        bail!("Refusing to stop an unverified Codex daemon");
    }
    let home = crate::auth::get_codex_home()?;
    let pid_path = home.join("app-server-daemon").join("daemon.pid");
    let record = std::fs::read_to_string(&pid_path)
        .context("Cannot verify the Codex daemon for this CODEX_HOME; close it from its own CLI")?;
    if parse_daemon_pid(&record) != Some(process.process_id) {
        bail!("Codex daemon belongs to another CODEX_HOME or changed; close it from its own CLI and retry");
    }

    // Use the executable of the verified running process, not an arbitrary
    // program found through PATH. Do not run shell text from the PID file.
    if process.executable_path.is_empty() {
        bail!("Cannot inspect the Codex daemon executable; close it from its own CLI and retry");
    }
    let executable = std::fs::canonicalize(&process.executable_path)
        .context("Cannot resolve the running Codex daemon executable")?;
    let package = std::fs::canonicalize(home.join("packages").join("app-server-daemon"))
        .context("Cannot verify the managed Codex installation")?;
    let executable_text = super::normalize_windows_path(&executable.to_string_lossy());
    let package_text = super::normalize_windows_path(&package.to_string_lossy());
    if !executable_text.starts_with(&format!("{}\\", package_text.trim_end_matches('\\')))
        || !executable_text.ends_with("\\codex.exe")
    {
        bail!("Daemon executable is outside this CODEX_HOME's managed installation; close it from its own CLI");
    }

    // Recheck the current snapshot immediately before invoking the lifecycle
    // command. The official stop command also validates the daemon record.
    let current = super::read_windows_codex_processes()?;
    if !current.iter().any(|candidate| {
        candidate.process_id == process.process_id
            && candidate.executable_path.eq_ignore_ascii_case(&process.executable_path)
            && candidate.command_line == process.command_line
            && is_managed_daemon(candidate)
    }) {
        bail!("Codex daemon changed during close; retry without changing credentials");
    }

    let mut child = Command::new(executable)
        .creation_flags(super::CREATE_NO_WINDOW)
        .args(["app-server", "daemon", "stop"])
        .env("CODEX_HOME", &home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Could not launch the Codex daemon stop command")?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => bail!("Codex daemon stop failed; credentials were not switched"),
            Ok(None) if started.elapsed() < Duration::from_secs(15) => {
                std::thread::sleep(Duration::from_millis(100));
            }
            outcome => {
                let _ = child.kill();
                let _ = child.wait();
                if let Err(error) = outcome {
                    return Err(error).context("Could not wait for the Codex daemon stop command");
                }
                bail!("Codex daemon stop timed out; credentials were not switched");
            }
        }
    }
    if super::read_windows_codex_processes()?
        .iter()
        .any(|candidate| candidate.process_id == process.process_id)
    {
        bail!("Codex daemon did not exit; credentials were not switched");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{is_managed_daemon, parse_daemon_pid, windows_arguments};
    use crate::commands::process::{classify_windows_codex_processes, WindowsCodexProcess};

    fn process(command: &str) -> WindowsCodexProcess {
        WindowsCodexProcess {
            name: "codex.exe".into(),
            process_id: 421,
            parent_process_id: 1,
            command_line: command.into(),
            executable_path: r"C:\Users\Test\.codex\packages\app-server-daemon\current\bin\codex.exe".into(),
            main_window_title: String::new(),
        }
    }

    #[test]
    fn headless_managed_daemon_blocks_switching() {
        let daemon = process(r#""C:\Users\Test User\.codex\packages\app-server-daemon\current\bin\codex.exe" app-server --listen unix:// --managed-daemon"#);
        assert!(is_managed_daemon(&daemon));
        assert_eq!(classify_windows_codex_processes(&[daemon]), (vec![421], 0));
    }

    #[test]
    fn bundled_managed_daemon_is_not_lost_to_resources_filter() {
        let mut daemon = process(r#""C:\Program Files\Codex\resources\codex.exe" app-server --managed-daemon"#);
        daemon.executable_path = r"C:\Program Files\Codex\resources\codex.exe".into();
        assert_eq!(classify_windows_codex_processes(&[daemon]), (vec![421], 0));
        // A non-managed-installation executable is detected but the runtime
        // close path refuses to invoke it automatically.
    }

    #[test]
    fn daemon_inside_ide_named_directory_still_blocks_switching() {
        let daemon = process(r#""C:\Users\Test\.vscode\codex.exe" app-server --managed-daemon"#);
        assert_eq!(classify_windows_codex_processes(&[daemon]), (vec![421], 0));
    }

    #[test]
    fn does_not_match_prompts_help_probes_or_control_commands() {
        for command in [
            r#"codex.exe exec "explain app-server --managed-daemon""#,
            "codex.exe app-server --managed-daemon --help",
            "codex.exe app-server daemon stop --managed-daemon",
            "codex.exe app-server --managed-daemon-docs",
            "codex.exe app-server -- --managed-daemon",
            r#"codex.exe app-server -c "note=app-server --managed-daemon""#,
            "codex.exe app-server --listen --managed-daemon",
            "codex.exe app-server --config --managed-daemon",
            "codex.exe app-server",
            "codex.exe",
            "",
        ] {
            assert!(!is_managed_daemon(&process(command)), "{command}");
        }
    }

    #[test]
    fn exact_quoted_flags_and_unicode_paths_work() {
        let daemon = process(r#""C:\Users\利用者 名\codex.exe" "app-server" "--managed-daemon""#);
        assert!(is_managed_daemon(&daemon));
    }

    #[test]
    fn unrelated_process_names_never_match() {
        let mut candidate = process("codex.exe app-server --managed-daemon");
        for name in ["ChatGPT.exe", "powershell.exe", "codex-switcher.exe"] {
            candidate.name = name.into();
            assert!(!is_managed_daemon(&candidate));
        }
    }

    #[test]
    fn rejects_an_unrelated_executable_in_command_line() {
        assert!(!is_managed_daemon(&process("other.exe app-server --managed-daemon")));
    }

    #[test]
    fn trailing_spaces_and_escaped_config_quotes_work() {
        let daemon = process(r#"codex.exe app-server -c "label=\"example\"" --managed-daemon  "#);
        assert!(is_managed_daemon(&daemon));
        assert_eq!(windows_arguments(r#"codex.exe "a\\\"b""#).unwrap()[1], "a\\\"b");
    }

    #[test]
    fn unquoted_backslash_does_not_escape_argument_separator() {
        assert_eq!(
            windows_arguments(r"codex.exe C:\test\ next").unwrap(),
            vec!["codex.exe", "C:\\test\\", "next"]
        );
    }

    #[test]
    fn duplicate_daemon_pids_are_counted_once() {
        let daemon = process("codex.exe app-server --managed-daemon");
        assert_eq!(
            classify_windows_codex_processes(&[daemon.clone(), daemon]),
            (vec![421], 0)
        );
    }

    #[test]
    fn unbalanced_quotes_are_not_guessed() {
        assert!(windows_arguments(r#""C:\Test\codex.exe app-server --managed-daemon"#).is_none());
    }

    #[test]
    fn reads_only_valid_pid_records() {
        assert_eq!(parse_daemon_pid(r#"{"pid":421,"processStartTime":"ignored"}"#), Some(421));
        for record in ["", "bad", "{}", r#"{"pid":"421"}"#, r#"{"pid":0}"#, r#"{"pid":-1}"#, r#"{"pid":4294967296}"#] {
            assert_eq!(parse_daemon_pid(record), None, "{record}");
        }
    }
}
