# Windows / WSL account-switch investigation

Observed on 2026-10-19 with the reported `v0.2.21-pine.1` installation and
Codex Desktop package `26.1013.4894.0`.

## Evidence

- The existing redacted diagnostic showed Windows `auth.json` matching the
  account selected in Pine. This does not establish Desktop's active account.
- Windows process metadata showed the packaged Desktop spawning `wsl.exe`
  with an app-server command.
- A read-only inspection of Linux process metadata identified Desktop-bundled
  app-server processes with the Desktop originator and a Linux-home `CODEX_HOME`.
- That Linux home and the Windows auth directory updated by Pine are distinct.
  Both contained the same saved identity at the inspection instant; no failing
  switch was performed during this investigation.

The destination mismatch explains how a successful Windows file update can leave
the WSL Desktop account unchanged. It does not establish keyring involvement or
prove that every reported failure has the same cause.

## Change

Switcher now supports an explicit existing account folder, including a local WSL
UNC directory. Changing it clears the selected marker without copying auth files.
An already selected account can be reapplied through the main account card; the
backend no longer returns success based only on that marker.

Windows switches targeting WSL check for remaining Codex processes in that
distribution. Unknown state and timeout block switching. Active-account token
refreshes reuse live credentials when Codex is running or its state is unknown;
inactive-account refreshes do not require Codex to stop. The check does not stop
those processes. Writes to WSL require an existing `auth.json` and retain its
Linux permissions.

The optional launcher uses the already installed executable with a child-only
`CODEX_HOME` override. It is a workaround for the destination mismatch, not a
replacement binary and not a backport of the new process guards.

## Validation and remaining acceptance

- Frontend TypeScript/Vite build passed.
- Existing Node suite: 32 passed.
- Actual auth/storage/type source modules in an isolated Rust harness: 18 passed,
  including three new destination/validation regressions. This harness excludes
  Tauri and the account-command and token-refresh modules.
- The same source modules and tests passed a Windows MSVC target `cargo check`.
  This was cross-target type checking, not execution on Windows.
- Windows PowerShell 5.1 launcher `-CheckOnly` confirmed the installed executable
  and target auth file are accessible, and detected that Pine and WSL Codex are
  still running. Its WSL probe uses LF on stdin to avoid PowerShell's CRLF
  producing a Linux shell syntax error.
- Full native Tauri build, installer creation, and switching real accounts after
  Desktop restart remain unverified. No release was published, installed app
  replaced, real account switched, or Desktop/CLI process stopped.

For runtime acceptance, close Codex and Pine, launch with the selected WSL home,
switch to the other saved account, reopen Desktop, and verify its displayed
account. Repeat in the reverse direction. Keep this runtime result separate from
the build, type-check, and synthetic-test results above.

## Follow-up: auth file unavailable after closing Desktop

The user reported the launcher's generic missing/inaccessible-auth error after
following the close-and-launch procedure. With Desktop running again, Windows
could open the same file read-only through both `wsl.localhost` and `wsl$`, and
the launcher check passed. The failed state has not been reproduced, so its
cause is not established. The earlier successful check only covered the
running-Desktop state, not access after shutdown.

Launcher version 2 checks WSL responsiveness before opening the auth file and
uses a read-only file handle instead of `Test-Path` for the auth check. On failure
it emits only classified metadata distinguishing a missing file, missing
directory, access denial, and other I/O errors. It does not recreate or copy
credentials, choose another home, or start Pine when auth access is unverified.

Version 2 was checked on Windows PowerShell 5.1 with the live path (readable),
a synthetic directory without `auth.json` (`file_not_found`, exit 1), and a
nonexistent synthetic directory (`directory_not_found`, exit 1). All used
`-CheckOnly`; no application launch or real credential change was performed.

## Follow-up: remaining WSL processes

After the auth-file check passed, the user reported the process blocker. A
read-only host inspection found a Linux Desktop (`/usr/lib/chatgpt/ChatGPT`),
its app-server, a separate app-server reparented to PID 1, and SSH-parented
app-servers. The inspected servers used the same Linux-home `CODEX_HOME`.
The Linux Desktop and PID-1-parented server had been running for roughly three
days. Windows Desktop exit alone therefore does not establish that this auth
store is unused. Sleeping state or an old start time is not proof of idle work.

Launcher version 3 reports bounded PID/name/parent metadata on this blocker,
without command lines, environment contents, or credential data. It does not
stop processes automatically. Terminating the remaining sessions requires
explicit approval because it can interrupt other Codex work, including this
chat. Process identity must be refreshed before any approved termination.

## Follow-up: opt-in shutdown launcher

The approved one-time shutdown log confirmed all targeted WSL processes stopped.
The subsequent separate PowerShell invocation failed, so that run did not confirm
Pine startup. The user also reported Windows Desktop closing at the same time.

Launcher version 4 integrates `-StopWslCodex` and `stop_wsl_codex.py` into the
Windows-side launcher itself, without another PowerShell launch. It requires a
typed `STOP` confirmation, snapshots recognized same-user processes for the
selected explicit home, opens pidfds, revalidates identities before signaling,
and sends only SIGTERM. A final process scan and the original launcher preflight
must both pass before Pine starts. `-CheckOnly` does not stop anything. This
does not shut down the WSL distribution or alter authentication files.

## Product integration: Windows and Ubuntu

The launcher workaround was runtime-verified by the user, including the actual
account change. That evidence does not validate the new Switcher binary.

The product now persists a device-local auth destination in Settings, offers a
directory picker, and can reset a stale invalid path from another device. The
existing account switch and close-confirmation commands include WSL process
checks/termination via an embedded Python helper. WSL PIDs stay separate from
Windows PIDs, and WSL termination is always SIGTERM with pidfd identity checks.
No username, distribution name, or repository location is hard-coded into this
product path. The selected WSL distribution needs Python 3 with pidfd support.

Native Ubuntu uses `/proc` to detect same-user Codex processes, including
headless app-servers previously ignored due to missing TTYs. Native Windows now
also blocks on headless roots and orphan bundled backends rather than assuming
they are stale. Supported Linux desktop roots are identified by their executable
and adjacent `resources/codex`, captured for the existing one-use reopen ticket,
and reopened with the selected `CODEX_HOME`. WSL desktops still reopen manually.

Local validation uses the actual auth/storage/process/reopen modules in an
isolated harness with a no-op Tauri command-attribute stub and real Tokio.
This checks Rust behavior and Windows-target compilation, not Tauri IPC or a
packaged application's lifecycle. Frontend build/tests and mocked Python helper
tests are also run. Full local Tauri compilation is blocked by uncached crates
and missing GTK/WebKit/system build tools. CI now includes Windows Rust tests and
the Python helper tests in addition to the existing Ubuntu Rust tests. CI has
not been run remotely and installers have not been built or published.

Observed local results: 53 isolated Rust tests, 37 Node tests, and 9 Python tests
passed; frontend production build passed; the isolated actual modules compiled
for `x86_64-pc-windows-msvc`. Windows-to-WSL stdin invocation of the embedded
helper's source returned a valid preview with exit 0 and empty stderr. No real
termination or credential write was performed while validating this integration.

Remaining acceptance: build the complete Windows/Ubuntu apps, test Settings and
close-and-switch via Tauri IPC, and confirm a real account change/reopen on each
native OS and Windows-to-WSL. File-based authentication is the supported contract;
OS keyring-backed Codex authentication is not covered.

## Automatic selection (PR #2 follow-up)

Manual folder entry is now an advanced override, not the primary setup flow.
Without that override, the resolver uses
observed running Linux/WSL Codex homes, the remembered device-local selection,
and existing login locations in that order. Multiple distinct live homes or
multiple existing logins require selection. An unknown probe result is not an
empty success. Stopped WSL distributions are not started merely for discovery;
without live evidence or a remembered selection, the user is asked to start the
desired environment and retry or select a folder.

The selected path and detection source are shown in Settings. Discovery is
rechecked on process polling (with a short cache) and before a close/switch under
the authentication-operation lock. The remembered path prevents closing a WSL
server from redirecting the subsequent write back to the native Windows folder.
Unavailable remembered folders block rather than silently falling back.
Discovery reads process-home metadata and file existence, not auth contents.
Its non-secret state is stored in `detected-codex-home.json` beside local settings.
Explicit manual selection bypasses failed detection.

Native Windows defaults are inferred from the normal login location; this does
not read native Windows processes' environment blocks. Native Codex detected
alongside observed WSL Codex therefore requires confirmation instead of claiming
that either home is authoritative. Custom native Windows per-process homes
remain a manual-override case.

The previous PR head `d5dc874` passed all six hosted checks, including full
Ubuntu Rust tests and Windows compilation/tests. That is not validation of this
follow-up. Follow-up local evidence: 66 isolated Rust tests, 38 Node tests, 13
Python tests, frontend build, and Windows-target compilation passed. A read-only
live probe returned the WSL home and also detected a native Windows Codex process,
which is intentionally treated as a conflict rather than guessed away.
No real account switch or termination was performed for this follow-up.

## Windows test feedback: inherited CODEX_HOME bypass

The user tested the Windows package with a Windows Codex app configured to use
WSL by default. They accepted the close confirmation and saw switch success,
but the Codex account did not change. Settings reported `CODEX_HOME` pointing
at the native Windows default. Read-only checks confirmed that the user-level
environment variable selects that default while a WSL Codex home also exists
in running processes. This is evidence of conflicting targets, not proof of
which account the Desktop actually loaded.

The prior resolver returned immediately for Switcher's inherited `CODEX_HOME`,
before discovering WSL homes. The WSL close/check helper is scoped to the selected
folder, so a native selection could close only Windows processes and write only
the native auth file. It did not establish that the WSL backend had switched.

`CODEX_HOME` is now a candidate, not a discovery bypass. A conflict with a running
home, an existing login without live evidence, or a different remembered home
requires explicit selection before closing or writing. Failed discovery still
blocks. A unique matching environment remains usable and is remembered across
close-and-switch. A confirmed manual folder remains the escape hatch for an
unrelated environment or unavailable discovery. No global environment variables
are removed or changed.

Regression tests cover the inherited-native-environment/live-WSL conflict,
agreement, conflicting existing logins, unknown WSL, remembered-home conflicts,
post-close retention, and manual override. Real Windows/WSL account switching
still needs to be retested with the rebuilt package.

## v0.2.22 usage refresh blocked by WSL Codex

The reported switch-blocking error also occurred on the usage-fetch path:
`get_usage_with_chatgpt_auth` refreshes expired credentials, and
`refresh_chatgpt_tokens_locked` unconditionally required WSL Codex to stop before
reaching the existing active-account guard. This blocked inactive accounts too.

The redundant unconditional check is removed. The active-account guard already
checks native and WSL processes and reuses reconciled live credentials on a
blocker or unknown state. Inactive accounts can refresh their saved credentials
without writing the selected Codex auth file. Actual account switching retains
its process checks. If the active account's live access token is also expired,
usage may still fail until Codex refreshes it or is closed for a safe refresh.

Validation: ten token-refresh tests passed in an isolated harness using the
actual auth/storage/process modules, a no-op Tauri command attribute, and no
HTTP/TLS requests. New tests cover inactive refresh independence, active WSL
blockers and unknown state, live-token reconciliation, and unchanged auth-file
bytes. Rust formatting and diff checks passed. The full application test command
was blocked by an uncached dependency (`chacha20poly1305`); packaged Windows/WSL
runtime behavior and a rebuilt installer remain unverified.

## Desktop app count in the status badge

The user confirmed that account switching works with the Windows
`0.2.23-usage.1` test package, but the badge displayed four Codex processes for
one visible app. The primary status now uses a separate `desktop_count` for
desktop app instances. Windows Electron children, CLI sessions, managed daemons,
and WSL backends do not increase this count. Linux desktop roots in the selected
WSL distribution are counted as apps in their own right.

The existing `count`, `pids`, `wsl_count`, and `can_switch` contract remains the
source of switching/closing decisions. If there are zero apps but remaining
blockers, the badge also shows `Background activity running` and retains the
close action. Missing desktop-count information is displayed as unavailable,
never substituted with a process count or an assumed zero. Process polling
updates the UI when the desktop count changes even if blocker counts stay equal.

Regression coverage includes one Windows app plus WSL backends, multiple apps,
CLI-only activity, Electron descendants, Linux desktop roots, missing display
counts, and the unchanged switching guard. The next Windows test package is
`0.2.23-usage.2`; its real-device app-count display still needs user confirmation.
