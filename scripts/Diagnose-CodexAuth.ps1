# Read-only diagnostics for Codex Switcher Pine. Windows PowerShell 5.1+.
# No network requests, credential writes, process termination, or Codex execution.
# Credentials are read only in memory for comparison. Only allowlisted metadata,
# booleans and local account-list positions are emitted; never raw JSON or errors.
[CmdletBinding()]
param(
    [ValidateRange(0, 120)][int]$ObserveSeconds = 60,
    [switch]$SelfTest
)
Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

function Field($Object, [string]$Name) {
    if ($null -eq $Object) { return $null }
    $p = $Object.PSObject.Properties[$Name]
    if ($null -ne $p) { return $p.Value }
    return $null
}
function Text($Value) {
    if ($Value -is [string]) { return $Value }
    return ''
}
function Json([string]$Content) {
    try { return ConvertFrom-Json -InputObject $Content -ErrorAction Stop }
    catch { return $null }
}
function ReadLocal([string]$Path) {
    try {
        # Do not access UNC paths, devices or relative paths.
        if ($Path -notmatch '^[A-Za-z]:[\\/]') {
            return [pscustomobject]@{ State = 'nonlocal_or_invalid'; Content = $null }
        }
        if (-not [IO.File]::Exists($Path)) {
            return [pscustomobject]@{ State = 'missing_or_inaccessible'; Content = $null }
        }
        $item = Get-Item -LiteralPath $Path -ErrorAction Stop
        if ($item.Length -gt 8388608) {
            return [pscustomobject]@{ State = 'too_large'; Content = $null }
        }
        return [pscustomobject]@{ State = 'read'; Content = [IO.File]::ReadAllText($Path) }
    } catch {
        return [pscustomobject]@{ State = 'read_error'; Content = $null }
    }
}
function JwtClaims([string]$Token) {
    try {
        if ($Token.Length -gt 131072) { return $null }
        $parts = $Token.Split('.')
        if ($parts.Count -ne 3) { return $null }
        $s = $parts[1].Replace('-', '+').Replace('_', '/')
        while (($s.Length % 4) -ne 0) { $s += '=' }
        return Json ([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($s)))
    } catch { return $null }
}
function Identity($Tokens, $ApiKey) {
    $key = Text $ApiKey
    if ($key.Length -gt 0) {
        return [pscustomobject]@{ Kind = 'api_key'; Key = $key; Workspace = ''; Subject = ''; Email = ''; IdToken = '' }
    }
    if ($null -eq $Tokens) { return $null }
    $id = Text (Field $Tokens 'id_token')
    $claims = JwtClaims $id
    $workspace = Text (Field $Tokens 'account_id')
    if ($workspace.Length -eq 0) {
        $workspace = Text (Field (Field $claims 'https://api.openai.com/auth') 'chatgpt_account_id')
    }
    return [pscustomobject]@{
        Kind = 'chatgpt'; Key = ''; Workspace = $workspace
        Subject = (Text (Field $claims 'sub'))
        Email = (Text (Field $claims 'email')).Trim().ToLowerInvariant()
        IdToken = $id
    }
}
function SameIdentity($A, $B) {
    # Null means unknown; a shared workspace ID alone never proves same user.
    if ($null -eq $A -or $null -eq $B) { return $null }
    if ($A.Kind -ne $B.Kind) { return $false }
    if ($A.Kind -eq 'api_key') { return ($A.Key -ceq $B.Key) }
    if ($A.Workspace.Length -gt 0 -and $B.Workspace.Length -gt 0) {
        if ($A.Workspace -cne $B.Workspace) { return $false }
        if ($A.Subject.Length -gt 0 -and $B.Subject.Length -gt 0) { return ($A.Subject -ceq $B.Subject) }
        if ($A.Email.Length -gt 0 -and $B.Email.Length -gt 0) { return ($A.Email -ceq $B.Email) }
    }
    if ($A.Workspace -ceq $B.Workspace -and $A.IdToken.Length -gt 0 -and $A.IdToken -ceq $B.IdToken) { return $true }
    return $null
}
function CredentialHints([string]$Content) {
    # Hints only, not a TOML resolver: profiles/managed/runtime overrides may win.
    $pattern = '(?m)^\s*cli_auth_credentials_store\s*=\s*["''](file|keyring|auto|ephemeral)["'']\s*(?:#.*)?$'
    return @([regex]::Matches($Content, $pattern) | ForEach-Object { $_.Groups[1].Value } | Select-Object -Unique)
}
function Summarize([string]$StoreText, [string]$AuthText) {
    $store = Json $StoreText
    $auth = Json $AuthText
    $accounts = @(Field $store 'accounts' | Where-Object { $null -ne $_ })
    $activeId = Text (Field $store 'active_account_id')
    $selected = $null
    $live = Identity (Field $auth 'tokens') (Field $auth 'OPENAI_API_KEY')
    $matches = @()
    $knownNonmatches = @()
    for ($i = 0; $i -lt $accounts.Count; $i++) {
        $a = $accounts[$i]
        $data = Field $a 'auth_data'
        if ($activeId.Length -gt 0 -and $activeId -ceq (Text (Field $a 'id'))) { $selected = $i + 1 }
        $same = SameIdentity $live (Identity $data (Field $data 'key'))
        if ($same -eq $true) { $matches += $i + 1 }
        elseif ($same -eq $false) { $knownNonmatches += $i + 1 }
    }
    $selectedMatches = $null
    if ($null -ne $selected) {
        if ($matches -contains $selected) { $selectedMatches = $true }
        elseif ($knownNonmatches -contains $selected) { $selectedMatches = $false }
    }
    $authMode = Text (Field $auth 'auth_mode')
    if ($authMode -notin @('chatgpt', 'apikey', 'chatgptAuthTokens', 'api_key')) { $authMode = 'missing_or_other' }
    return [pscustomobject][ordered]@{
        storeJsonParsed = ($null -ne $store)
        authJsonParsed = ($null -ne $auth)
        savedAccountCount = $accounts.Count
        selectedSavedPosition = $selected
        authMatchesSavedPositions = @($matches)
        authMatchesPineSelection = $selectedMatches
        authModeHint = $authMode
        hasTokenBlock = ($null -ne (Field $auth 'tokens'))
        hasApiKey = ((Text (Field $auth 'OPENAI_API_KEY')).Length -gt 0)
    }
}
function ProcessSummary {
    try {
        $items = @(Get-CimInstance Win32_Process -Filter "Name = 'Codex.exe' OR Name = 'ChatGPT.exe' OR Name = 'codex-switcher.exe'" -ErrorAction Stop)
        return [pscustomobject]@{
            state = 'read'; codexProcesses = @($items | Where-Object { $_.Name -ieq 'Codex.exe' }).Count
            chatgptProcesses = @($items | Where-Object { $_.Name -ieq 'ChatGPT.exe' }).Count
            switcherProcesses = @($items | Where-Object { $_.Name -ieq 'codex-switcher.exe' }).Count
            managedFlagSeen = @($items | Where-Object { (Text $_.CommandLine) -match '(?:^|\s)--managed-daemon(?:\s|$)' }).Count
            packagedCodexSeen = @($items | Where-Object { (Text $_.ExecutablePath) -match '\\WindowsApps\\OpenAI\.Codex_' }).Count
            credentialStoreHints = @(CredentialHints (($items | ForEach-Object {
                # Extract only a known enum; never emit raw command lines.
                $m = [regex]::Match((Text $_.CommandLine), 'cli_auth_credentials_store\s*=\s*\\?["'']?(file|keyring|auto|ephemeral)\b')
                if ($m.Success) { 'cli_auth_credentials_store="' + $m.Groups[1].Value + '"' }
            }) -join "`n"))
        }
    } catch { return [pscustomobject]@{ state = 'query_error' } }
}
function RunSelfTests {
    $script:Checks = 0
    function Assert([bool]$Condition, [string]$Name) {
        if (-not $Condition) { throw ('Self-test failed: ' + $Name) }
        $script:Checks++
    }
    function Token([string]$Subject, [string]$Email) {
        $c = @{ sub = $Subject; email = $Email } | ConvertTo-Json -Compress
        $b = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($c)).TrimEnd('=').Replace('+', '-').Replace('/', '_')
        return 'header.' + $b + '.signature'
    }
    $ta = @{ id_token = (Token 'person-A' 'private-a@example.invalid'); account_id = 'workspace-SECRET' }
    $tb = @{ id_token = (Token 'person-B' 'private-b@example.invalid'); account_id = 'workspace-SECRET' }
    $store = @{ active_account_id = 'saved-B'; accounts = @(
        @{ id = 'saved-A'; name = 'PRIVATE-LABEL'; auth_data = $ta },
        @{ id = 'saved-B'; name = 'OTHER-LABEL'; auth_data = $tb }
    ) } | ConvertTo-Json -Depth 8 -Compress
    $aa = @{ tokens = $ta; auth_mode = 'chatgpt' } | ConvertTo-Json -Depth 8 -Compress
    $ab = @{ tokens = $tb; auth_mode = 'chatgpt' } | ConvertTo-Json -Depth 8 -Compress
    $r = Summarize $store $aa
    Assert ($r.authMatchesPineSelection -eq $false) 'stale selected marker is not success'
    Assert ($r.authMatchesSavedPositions.Count -eq 1 -and $r.authMatchesSavedPositions[0] -eq 1) 'same workspace different users'
    Assert ($r.selectedSavedPosition -eq 2) 'selection is only a numbered position'
    Assert ((Summarize $store $ab).authMatchesPineSelection -eq $true) 'target user matches'
    Assert ($null -eq (Summarize $store '{}').authMatchesPineSelection) 'absent auth is unknown'
    Assert (-not (Summarize $store 'bad-json').authJsonParsed) 'malformed auth is not parsed'
    Assert (-not (Summarize 'bad-json' $aa).storeJsonParsed) 'malformed store is not parsed'
    Assert ($null -eq (JwtClaims 'malformed')) 'malformed JWT'
    Assert ($null -eq (JwtClaims 'header.%%.signature')) 'invalid JWT base64'
    $out = $r | ConvertTo-Json -Depth 8 -Compress
    foreach ($secret in @('private-a@example.invalid', 'private-b@example.invalid', 'PRIVATE-LABEL', 'OTHER-LABEL', 'saved-B', 'person-A', 'workspace-SECRET', $ta.id_token)) {
        Assert (-not $out.Contains($secret)) 'redaction'
    }
    Assert ((@(CredentialHints 'cli_auth_credentials_store = "keyring" # PRIVATE')).Count -eq 1) 'known config enum'
    Assert ((@(CredentialHints '# cli_auth_credentials_store = "auto"')).Count -eq 0) 'comment ignored'
    Assert ((@(CredentialHints 'cli_auth_credentials_store = "PRIVATE-UNKNOWN"')).Count -eq 0) 'unknown enum omitted'
    Assert ((@(CredentialHints "[profiles.local]`ncli_auth_credentials_store = 'auto'")).Count -eq 1) 'profile declaration is a hint'
    $a = Identity (Json ($ta | ConvertTo-Json)) $null
    $b = Identity (Json ($tb | ConvertTo-Json)) $null
    Assert ((SameIdentity $a $b) -eq $false) 'distinct users in workspace'
    $b = Identity (Json ($ta | ConvertTo-Json)) $null
    $b.Workspace = 'other-workspace'
    Assert ((SameIdentity $a $b) -eq $false) 'same user different workspace'
    $a = Identity (Json '{"account_id":"shared"}') $null
    $b = Identity (Json '{"account_id":"shared"}') $null
    Assert ($null -eq (SameIdentity $a $b)) 'workspace alone is insufficient'
    Assert ((SameIdentity (Identity $null 'PRIVATE-KEY') (Identity $null 'PRIVATE-KEY')) -eq $true) 'API key comparison'
    Assert ((SameIdentity (Identity $null 'PRIVATE-KEY') (Identity $null 'OTHER-KEY')) -eq $false) 'different API keys'
    $keyStore = '{"active_account_id":"key-slot","accounts":[{"id":"key-slot","auth_data":{"key":"PRIVATE-KEY"}}]}'
    $kr = Summarize $keyStore '{"OPENAI_API_KEY":"PRIVATE-KEY"}'
    Assert ($kr.authMatchesPineSelection -eq $true) 'API key selected match'
    Assert (-not ($kr | ConvertTo-Json -Compress).Contains('PRIVATE-KEY')) 'API key never emitted'
    Assert ((ReadLocal '\\server\share\auth.json').State -eq 'nonlocal_or_invalid') 'UNC rejected before IO'
    Assert ((ReadLocal 'relative/auth.json').State -eq 'nonlocal_or_invalid') 'relative path rejected'
    Write-Output ('Self-tests passed: ' + $script:Checks)
}

if ($SelfTest) { RunSelfTests; exit 0 }
try {
    if ($env:OS -ne 'Windows_NT') { throw 'unsupported_os' }
    $userHome = [Environment]::GetFolderPath('UserProfile')
    $storePath = Join-Path $userHome '.codex-switcher\accounts.json'
    $homes = @([pscustomobject]@{ Label = 'default_codex_home'; Path = (Join-Path $userHome '.codex') })
    $envReport = @()
    foreach ($scope in @('Process', 'User', 'Machine')) {
        $value = [Environment]::GetEnvironmentVariable('CODEX_HOME', $scope)
        $present = -not [string]::IsNullOrWhiteSpace($value)
        $envReport += [pscustomobject]@{ scope = $scope; codexHomeSet = $present }
        if ($present -and $value -match '^[A-Za-z]:[\\/]') {
            $path = [IO.Path]::GetFullPath($value).TrimEnd('\', '/')
            if (@($homes | Where-Object { $_.Path.TrimEnd('\', '/') -ieq $path }).Count -eq 0) {
                $homes += [pscustomobject]@{ Label = ('environment_' + $scope.ToLowerInvariant()); Path = $path }
            }
        }
    }
    $homeHints = @()
    foreach ($homeEntry in $homes) {
        $cfg = ReadLocal (Join-Path $homeEntry.Path 'config.toml')
        $homeHints += [pscustomobject]@{
            source = $homeEntry.Label; configFileState = $cfg.State
            declaredCredentialStores = @(CredentialHints (Text $cfg.Content))
        }
    }
    $packages = @()
    try {
        $packages = @(Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction Stop | ForEach-Object {
            $v = [string]$_.Version
            if ($v -match '^\d+(?:\.\d+){1,4}$') { $v }
        })
    } catch { }
    $before = ProcessSummary
    Write-Host 'Read-only diagnostic started. In Pine, switch accounts ONCE as usual now.'
    Write-Host ('Observe for ' + $ObserveSeconds + ' seconds. Do not paste any auth files. Wait for the JSON report.')
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $events = @()
    $previous = @{}
    do {
        $storeFile = ReadLocal $storePath
        foreach ($homeEntry in $homes) {
            $authFile = ReadLocal (Join-Path $homeEntry.Path 'auth.json')
            $summary = Summarize (Text $storeFile.Content) (Text $authFile.Content)
            $summary | Add-Member -NotePropertyName storeFileState -NotePropertyValue $storeFile.State
            $summary | Add-Member -NotePropertyName authFileState -NotePropertyValue $authFile.State
            $encoded = $summary | ConvertTo-Json -Depth 6 -Compress
            if (-not $previous.ContainsKey($homeEntry.Label) -or $previous[$homeEntry.Label] -cne $encoded) {
                $events += [pscustomobject]@{ seconds = [math]::Round($watch.Elapsed.TotalSeconds, 1); source = $homeEntry.Label; state = $summary }
                $previous[$homeEntry.Label] = $encoded
            }
        }
        if ($watch.Elapsed.TotalSeconds -ge $ObserveSeconds) { break }
        Start-Sleep -Milliseconds 500
    } while ($true)
    $report = [pscustomobject][ordered]@{
        diagnosticVersion = 1
        powershell = $PSVersionTable.PSVersion.ToString()
        windowsBuild = [Environment]::OSVersion.Version.ToString()
        installedCodexPackageVersions = @($packages)
        codexHomeEnvironment = @($envReport)
        configHints = @($homeHints)
        processesBefore = $before
        processesAfter = (ProcessSummary)
        sampledStates = @($events)
        limitations = 'File-state observation only. Store hints are not resolved config. OS credential stores and Desktop login were not read. Account positions refer to local saved order, not UI sorting. Short changes between samples can be missed. Null means unknown.'
    }
    Write-Output '--- BEGIN REDACTED CODEX AUTH DIAGNOSTIC ---'
    $report | ConvertTo-Json -Depth 12
    Write-Output '--- END REDACTED CODEX AUTH DIAGNOSTIC ---'
} catch {
    # Do not print exceptions: JSON/IO errors can contain credentials or paths.
    Write-Output '{"diagnosticVersion":1,"state":"diagnostic_failed_no_private_details_emitted"}'
    exit 1
}
