[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$WslHome,
    [string]$SwitcherPath = (Join-Path $env:LOCALAPPDATA 'Codex Switcher\codex-switcher.exe'),
    [switch]$CheckOnly,
    [switch]$StopWslCodex
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

try {
    if ($WslHome -notmatch '^\\\\(?:wsl\.localhost|wsl\$)\\([A-Za-z0-9][A-Za-z0-9_.-]*)\\(.+)$') {
        throw 'Use a local WSL UNC folder, for example \\wsl.localhost\Ubuntu\home\your-user\.codex.'
    }
    $distribution = $Matches[1]
    $linuxHome = '/' + $Matches[2].Replace('\', '/').TrimEnd('/')
    if (-not (Test-Path -LiteralPath $SwitcherPath -PathType Leaf)) {
        throw 'Pine executable was not found. Supply -SwitcherPath with its installed location.'
    }
    $running = @(Get-Process -Name 'codex-switcher' -ErrorAction SilentlyContinue)
    if ($running.Count -gt 0 -and -not $CheckOnly) {
        throw 'Quit Pine from its tray menu first. Closing its window alone may leave it running.'
    }
    if ($StopWslCodex -and -not $CheckOnly) {
        $authCheck = [System.IO.File]::Open((Join-Path $WslHome 'auth.json'), [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
        $authCheck.Dispose()
        Write-Warning 'This stops WSL Codex sessions using the selected home, including their Linux Desktop. Windows Codex and this chat may disconnect. Other WSL applications are not stopped.'
        if ((Read-Host 'Save your work. Type STOP to stop WSL Codex and launch Pine') -cne 'STOP') {
            throw 'Cancelled; no processes were stopped.'
        }
        $stopSource = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'stop_wsl_codex.py') -Raw
        $homeJson = ConvertTo-Json -InputObject $linuxHome -Compress
        $stopSource = "import sys`nsys.argv = ['stop_wsl_codex.py', '--home', " + $homeJson + ", '--stop']`n" + $stopSource
        $stopStart = New-Object System.Diagnostics.ProcessStartInfo
        $stopStart.FileName = 'wsl.exe'
        $stopStart.Arguments = '--distribution ' + $distribution + ' --exec /usr/bin/python3 -'
        $stopStart.UseShellExecute = $false
        $stopStart.CreateNoWindow = $true
        $stopStart.RedirectStandardInput = $true
        $stopStart.RedirectStandardOutput = $true
        $stopStart.RedirectStandardError = $true
        $stopProcess = [System.Diagnostics.Process]::Start($stopStart)
        try {
            $stopOutputTask = $stopProcess.StandardOutput.ReadToEndAsync()
            $stopErrorTask = $stopProcess.StandardError.ReadToEndAsync()
            $stopProcess.StandardInput.Write($stopSource.Replace("`r`n", "`n") + "`n")
            $stopProcess.StandardInput.Close()
            if (-not $stopProcess.WaitForExit(30000)) {
                $stopProcess.Kill()
                throw 'Shutdown timed out; Pine was not launched. No forced termination will be attempted.'
            }
            $stopOutput = $stopOutputTask.GetAwaiter().GetResult()
            Write-Output $stopOutput
            if ($stopProcess.ExitCode -ne 0) { throw 'WSL shutdown was not verified; Pine was not launched.' }
            $records = @($stopOutput -split '[\r\n]+' | Where-Object { $_.Length -gt 0 })
            if ($records.Count -eq 0) { throw 'Shutdown returned no verification result.' }
            $verified = $records[-1] | ConvertFrom-Json
            if ($verified.status -ne 'stopped' -or @($verified.remainingTargetPids).Count -ne 0 -or @($verified.blockingProcesses).Count -ne 0) {
                throw 'WSL Codex processes remain; Pine was not launched.'
            }
        } finally {
            $stopProcess.Dispose()
        }
    }
    $probe = @'
test -r /proc/self/comm || exit 2
found=0
for entry in /proc/[0-9]*/comm; do
    test -e "$entry" || continue
    IFS= read -r name < "$entry" || exit 2
    case "$name" in
        codex|codex-*)
            found=$((found + 1))
            test "$found" -le 32 || exit 2
            directory=${entry%/comm}
            pid=${directory##*/}
            parent=
            while read -r key value rest; do
                case "$key" in PPid:) parent=$value; break;; esac
            done < "$directory/status"
            case "$parent" in ''|*[!0-9]*) exit 2;; esac
            parent_name=
            IFS= read -r parent_name < "/proc/$parent/comm" || exit 2
            case "$parent_name" in
                ChatGPT) kind=desktop;;
                sshd) kind=ssh;;
                codex) kind=codex;;
                *) if test "$parent" -eq 1; then kind=orphan; else kind=other; fi;;
            esac
            printf '%s %s %s %s\n' "$pid" "$name" "$parent" "$kind"
            ;;
    esac
done
test "$found" -eq 0 || exit 3
'@
    $probe = $probe.Replace("`r`n", "`n")
    $probeStart = New-Object System.Diagnostics.ProcessStartInfo
    $probeStart.FileName = 'wsl.exe'
    $probeStart.Arguments = '--distribution ' + $distribution + ' --exec /bin/sh'
    $probeStart.UseShellExecute = $false
    $probeStart.CreateNoWindow = $true
    $probeStart.RedirectStandardInput = $true
    $probeStart.RedirectStandardOutput = $true
    $probeStart.RedirectStandardError = $true
    $probeProcess = [System.Diagnostics.Process]::Start($probeStart)
    try {
        $probeProcess.StandardInput.Write($probe + "`n")
        $probeProcess.StandardInput.Close()
        if (-not $probeProcess.WaitForExit(10000)) {
            $probeProcess.Kill()
            throw 'Timed out checking WSL Codex processes.'
        }
        $probeExitCode = $probeProcess.ExitCode
        $probeOutput = $probeProcess.StandardOutput.ReadToEnd()
    } finally {
        $probeProcess.Dispose()
    }
    if ($probeExitCode -ne 0 -and $probeExitCode -ne 3) {
        throw 'Could not verify that WSL Codex processes have stopped.'
    }
    $blockers = @()
    foreach ($line in ($probeOutput -split '[\r\n]+')) {
        if ($line.Length -eq 0) { continue }
        if ($line -notmatch '^([1-9][0-9]*) (codex(?:-[a-z0-9_-]+)?) ([0-9]+) (desktop|ssh|codex|orphan|other)$') {
            throw 'WSL returned an unrecognized process record; launch was cancelled.'
        }
        $blockers += [pscustomobject]@{ pid = [uint32]$Matches[1]; name = $Matches[2]; parentPid = [uint32]$Matches[3]; parentKind = $Matches[4] }
    }
    if (($probeExitCode -eq 3) -ne ($blockers.Count -gt 0)) {
        throw 'WSL process records did not match the check result; launch was cancelled.'
    }
    $authPath = Join-Path $WslHome 'auth.json'
    $authState = 'readable'
    try {
        $authStream = [System.IO.File]::Open($authPath, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, ([System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete))
        $authStream.Dispose()
    } catch {
        $reason = $_.Exception
        while ($null -ne $reason.InnerException) { $reason = $reason.InnerException }
        if ($reason -is [System.IO.FileNotFoundException]) { $authState = 'file_not_found' }
        elseif ($reason -is [System.IO.DirectoryNotFoundException]) { $authState = 'directory_not_found' }
        elseif ($reason -is [System.UnauthorizedAccessException]) { $authState = 'access_denied' }
        elseif ($reason -is [System.IO.IOException]) { $authState = 'io_error' }
        else { $authState = 'unknown_error' }
    }
    $diagnostic = [pscustomobject]@{
        launcherVersion = 4
        wslResponded = $true
        directoryVisible = [System.IO.Directory]::Exists($WslHome)
        authFileState = $authState
        pineInstalled = $true
        pineMustExitBeforeLaunch = ($running.Count -gt 0)
        wslCodexRunning = ($probeExitCode -eq 3)
        blockingProcesses = @($blockers)
        credentialsChanged = $false
    }
    if ($CheckOnly -or $authState -ne 'readable' -or $probeExitCode -eq 3) {
        Write-Output '--- BEGIN PINE WSL LAUNCH CHECK ---'
        $diagnostic | ConvertTo-Json -Depth 4 -Compress | Write-Output
        Write-Output '--- END PINE WSL LAUNCH CHECK ---'
    }
    if ($authState -ne 'readable') {
        throw ('WSL responded, but auth.json is not readable (' + $authState + '). No files were created or copied. Share the launch-check JSON above.')
    }
    if ($CheckOnly) {
        exit 0
    }
    if ($probeExitCode -eq 3) {
        throw 'WSL Codex processes remain (see blockingProcesses above). A Linux Desktop, SSH session, or background app-server can remain after Windows Codex closes. Launch was cancelled; no processes were stopped.'
    }
    $start = New-Object System.Diagnostics.ProcessStartInfo
    $start.FileName = $SwitcherPath
    $start.UseShellExecute = $false
    $start.EnvironmentVariables['CODEX_HOME'] = $WslHome
    [void][System.Diagnostics.Process]::Start($start)
    Write-Output 'Pine started with the WSL account folder. Switch to the desired account, then open Codex and verify the account shown there.'
} catch {
    Write-Error -Message $_.Exception.Message -ErrorAction Continue
    exit 1
}
