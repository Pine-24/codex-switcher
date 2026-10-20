use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(super) fn output(program: &str, arguments: &[&str], input: Option<String>) -> Result<Vec<u8>> {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .args(arguments)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Could not start account-folder detection")?;
    let reader = child.stdout.take().context("Missing detection output")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.take(262145).read_to_end(&mut bytes).map(|_| bytes)
    });
    let writer = if let Some(input) = input {
        let mut writer = child.stdin.take().context("Missing detection input")?;
        Some(std::thread::spawn(move || {
            writer.write_all(input.as_bytes())
        }))
    } else {
        None
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Account-folder detection timed out");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    anyhow::ensure!(
        status.success(),
        "Account-folder detection failed; no fallback was selected"
    );
    if let Some(writer) = writer {
        writer
            .join()
            .map_err(|_| anyhow::anyhow!("Detection input failed"))??;
    }
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Detection output failed"))??;
    anyhow::ensure!(
        bytes.len() <= 262144,
        "Account-folder detection output was too large"
    );
    Ok(bytes)
}

#[cfg(windows)]
pub(super) fn discover_wsl(
    discovery: &mut super::home_detection::Discovery,
    native: &std::path::Path,
) -> Result<()> {
    use super::home_detection::HomeCandidate;
    #[derive(serde::Deserialize)]
    struct WindowsState {
        registered: Vec<String>,
        native_running: bool,
    }
    let script = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss'
$names = @()
if (Test-Path $key) {
    $names = @(Get-ChildItem $key | ForEach-Object { (Get-ItemProperty $_.PSPath).DistributionName } | Where-Object { $_ })
}
$native = @(Get-CimInstance Win32_Process -Filter "Name = 'codex.exe'" | Where-Object { $_.CommandLine -notlike '*--type=*' })
@{ registered = @($names); native_running = ($native.Count -gt 0) } | ConvertTo-Json -Compress
"#;
    let state: WindowsState = serde_json::from_slice(&output(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", script],
        None,
    )?)?;
    if state.registered.is_empty() {
        return Ok(());
    }
    let running = decode_distributions(&output(
        "wsl.exe",
        &["--list", "--running", "--quiet"],
        None,
    )?)?;
    anyhow::ensure!(
        running.len() <= 16,
        "Too many running WSL distributions for automatic detection"
    );
    if running.len() < state.registered.len() && !state.native_running {
        discovery.unresolved = Some("Some WSL distributions are stopped. Start the Codex environment you want to use and retry detection, or select its folder.".into());
    }
    for distribution in running {
        anyhow::ensure!(
            super::switcher::wsl_distribution(&format!(r"\\wsl.localhost\{distribution}\home"))
                .is_some(),
            "Unsupported WSL distribution name"
        );
        let bytes = output(
            "wsl.exe",
            &[
                "--distribution",
                &distribution,
                "--exec",
                "/bin/sh",
                "-c",
                WSL_HOME_PROBE,
            ],
            None,
        )?;
        let (home, exists, live) = parse_home_probe(&bytes)?;
        if exists {
            discovery.available.push(HomeCandidate {
                path: unc_home(&distribution, &home)?,
                source: format!("Existing WSL login ({distribution})"),
            });
        }
        if live {
            let source = format!(
                "import sys\nsys.argv = ['stop_wsl_codex.py', '--discover']\n{}",
                include_str!("../../../scripts/stop_wsl_codex.py")
            );
            #[derive(serde::Deserialize)]
            struct Homes {
                status: String,
                homes: Vec<String>,
            }
            let homes: Homes = serde_json::from_slice(&output(
                "wsl.exe",
                &[
                    "--distribution",
                    &distribution,
                    "--exec",
                    "/usr/bin/python3",
                    "-",
                ],
                Some(source),
            )?)?;
            anyhow::ensure!(
                homes.status == "discovered",
                "WSL did not return verified account folders"
            );
            for path in homes.homes {
                discovery.live.push(HomeCandidate {
                    path: unc_home(&distribution, &path)?,
                    source: format!("Running Codex in WSL ({distribution})"),
                });
            }
        }
    }
    if state.native_running && !discovery.live.is_empty() {
        discovery.live.push(HomeCandidate {
            path: native.to_string_lossy().into_owned(),
            source: "Native Windows Codex also running; confirm its folder".into(),
        });
    }
    Ok(())
}

const WSL_HOME_PROBE: &str = r#"
test -r /proc/self/comm || exit 2
case "$HOME" in /*) ;; *) exit 2;; esac
live=0
for entry in /proc/[0-9]*/comm; do
    test -e "$entry" || continue
    IFS= read -r name < "$entry" || exit 2
    case "$name" in
        codex-switcher*) ;;
        codex|codex-*) live=1;;
    esac
done
exists=0
test ! -f "$HOME/.codex/auth.json" || exists=1
printf '%s\000%s\000%s\000' "$HOME/.codex" "$exists" "$live"
"#;

fn decode_distributions(bytes: &[u8]) -> Result<Vec<String>> {
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.contains(&0) {
        anyhow::ensure!(bytes.len() % 2 == 0, "Malformed WSL distribution list");
        String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        )?
    } else {
        String::from_utf8(bytes.to_vec())?
    };
    Ok(text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
}

fn parse_home_probe(bytes: &[u8]) -> Result<(String, bool, bool)> {
    let parts: Vec<_> = bytes.split(|byte| *byte == 0).collect();
    anyhow::ensure!(
        parts.len() == 4 && parts[3].is_empty(),
        "Malformed WSL home probe"
    );
    let flag = |value: &[u8]| -> Result<bool> {
        match value {
            b"0" => Ok(false),
            b"1" => Ok(true),
            _ => anyhow::bail!("Invalid WSL probe flag"),
        }
    };
    Ok((
        String::from_utf8(parts[0].to_vec())?,
        flag(parts[1])?,
        flag(parts[2])?,
    ))
}

fn unc_home(distribution: &str, home: &str) -> Result<String> {
    anyhow::ensure!(
        home.starts_with('/')
            && !home.contains(['\\', '\0'])
            && !home.split('/').any(|part| matches!(part, "." | "..")),
        "Invalid Linux account folder"
    );
    Ok(format!(
        r"\\wsl.localhost\{}\{}",
        distribution,
        home.trim_start_matches('/').replace('/', "\\")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_utf16_and_utf8_wsl_names() {
        let encoded: Vec<_> = "Ubuntu\r\nDebian\r\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(
            decode_distributions(&encoded).unwrap(),
            vec!["Ubuntu", "Debian"]
        );
        assert_eq!(decode_distributions(b"Ubuntu\n").unwrap(), vec!["Ubuntu"]);
        assert!(decode_distributions(&[0, 1, 2]).is_err());
    }
    #[test]
    fn home_probe_requires_all_fields_and_does_not_accept_unknown_flags() {
        assert_eq!(
            parse_home_probe(b"/home/test/.codex\0\x31\0\x30\0").unwrap(),
            ("/home/test/.codex".into(), true, false)
        );
        assert!(parse_home_probe(b"/home/test\0").is_err());
        assert!(parse_home_probe(b"/home/test\0maybe\0\x30\0").is_err());
    }
    #[test]
    fn unc_conversion_preserves_spaces_and_rejects_ambiguous_separators() {
        assert_eq!(
            unc_home("Ubuntu", "/home/test user/.codex").unwrap(),
            r"\\wsl.localhost\Ubuntu\home\test user\.codex"
        );
        for home in ["relative", "/home/../other", "/home/back\\slash"] {
            assert!(unc_home("Ubuntu", home).is_err());
        }
    }
}
