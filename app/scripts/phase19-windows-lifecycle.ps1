param(
    [Parameter(Mandatory)][ValidateSet('Clean','Upgrade')][string]$Scenario,
    [Parameter(Mandatory)][ValidateSet('HostedRunner','IsolatedGuest')][string]$Environment,
    [Parameter(Mandatory)][string]$Installers,
    [Parameter(Mandatory)][string]$Evidence,
    [switch]$ExerciseRecovery
)

$ErrorActionPreference = 'Stop'
if ($Environment -eq 'HostedRunner' -and
    ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted')) {
    throw 'Expected a GitHub-hosted Windows runner.'
}
$os = Get-CimInstance Win32_OperatingSystem
if ($os.ProductType -notin 1, 3 -or
    [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString() -ne 'X64') {
    throw 'Expected an x64 Windows guest.'
}
if ($Environment -eq 'IsolatedGuest' -and $ExerciseRecovery -and
    ($os.ProductType -ne 1 -or [int]$os.BuildNumber -lt 22000)) {
    throw 'Phase 20 desktop qualification requires Windows 11 x64.'
}
$installDirectory = Join-Path $env:LOCALAPPDATA 'SkillVerse AI'
$dataDirectory = Join-Path $env:APPDATA 'ai.skillverse.codex'
# The packaged release uses the normal application data directory.
$database = Join-Path $dataDirectory 'skillverse.sqlite3'
if ($env:APPDATA -ne [Environment]::GetFolderPath('ApplicationData') -or
    $env:LOCALAPPDATA -ne [Environment]::GetFolderPath('LocalApplicationData') -or
    $env:WEBVIEW2_USER_DATA_FOLDER -or $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER -or
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -or $env:WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER) {
    throw 'Expected default profile folders and WebView2 settings for installer lifecycle tests.'
}
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\SkillVerse AI'
$machineKey = 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\SkillVerse AI'
$machineKey32 = 'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\SkillVerse AI'
$codexDirectory = Join-Path $env:USERPROFILE '.codex'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if ((Test-Path $installDirectory) -or (Test-Path $dataDirectory) -or (Test-Path $uninstallKey) -or
    (Test-Path $machineKey) -or (Test-Path $machineKey32) -or (Test-Path $codexDirectory) -or
    $env:CODEX_HOME -or $env:OPENAI_API_KEY -or $env:DEEPSEEK_API_KEY -or
    (Get-Command codex -ErrorAction SilentlyContinue) -or
    (Get-Process skillverse-ai -ErrorAction SilentlyContinue)) {
    throw 'Expected a clean test profile without SkillVerse, Codex, or provider credentials.'
}
if (Test-Path -LiteralPath $Evidence) { throw 'Provide a new evidence directory.' }
$null = New-Item -ItemType Directory -Path $Evidence
$evidenceRoot = (Resolve-Path -LiteralPath $Evidence).Path
$temp = Join-Path $evidenceRoot 'guest-temp'
$null = New-Item -ItemType Directory -Path $temp
$previousTemp = $env:TEMP
$previousTmp = $env:TMP
$env:TEMP = $temp
$env:TMP = $temp
$report = [ordered]@{
    scenario = $Scenario
    environment = $Environment
    windows = $os.Caption
    build = $os.BuildNumber
    image = $env:ImageVersion
    elevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    installers = [ordered]@{}
    steps = [ordered]@{}
    result = 'FAILED'
}

function Test-Database($mode, $baseline) {
    $argsList = @((Join-Path $PSScriptRoot 'phase19-guest-db.py'), $mode, $database)
    if ($baseline) { $argsList += @('--baseline', $baseline) }
    $json = & python @argsList
    if ($LASTEXITCODE) { throw "Database $mode failed." }
    $json | ConvertFrom-Json
}

function Install-Candidate($version, $stage) {
    $report.stage = $stage
    $setup = Join-Path $Installers "SkillVerse AI_${version}_x64-setup.exe"
    $process = Start-Process -FilePath $setup -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
    $entry = $report.steps[$stage] = [ordered]@{ exitCode = $process.ExitCode }
    if ($process.ExitCode) { throw "Installer failed at $stage." }
    $exe = Get-Item -LiteralPath (Join-Path $installDirectory 'skillverse-ai.exe')
    $uninstaller = Join-Path $installDirectory 'uninstall.exe'
    $registered = Get-ItemProperty -LiteralPath $uninstallKey
    if ($registered.DisplayName -ne 'SkillVerse AI' -or $registered.DisplayVersion -ne $version -or
        $registered.InstallLocation.Trim('"') -ne $installDirectory -or
        $registered.UninstallString.Trim('"') -ne $uninstaller -or -not (Test-Path $uninstaller) -or
        $exe.VersionInfo.ProductVersion -ne $version -or (Test-Path $machineKey) -or (Test-Path $machineKey32)) {
        throw 'Expected matching installed files and current-user uninstall registration.'
    }
    $exeHash = (Get-FileHash -LiteralPath $exe.FullName).Hash
    if ($version -eq '0.1.2' -and
        $exeHash -ne 'ad2b3c0a8df4998dcd14908247dc291ed441ad04b63ef5ad544e4a58b5e888dc') {
        throw 'The installed 0.1.2 application differs from the verified NSIS payload.'
    }
    $entry.version = $version
    $entry.registryScope = 'HKCU'
    $entry.files = @(Get-ChildItem -LiteralPath $installDirectory -File -Recurse | ForEach-Object {
        [ordered]@{ path = $_.FullName.Substring($installDirectory.Length + 1); bytes = $_.Length
            sha256 = (Get-FileHash -LiteralPath $_.FullName).Hash.ToLowerInvariant() }
    })
}

function Start-And-Close($stage, [switch]$Terminate) {
    $report.stage = $stage
    $exe = Join-Path $installDirectory 'skillverse-ai.exe'
    $process = Start-Process -FilePath $exe -WorkingDirectory $installDirectory -PassThru
    $entry = $report.steps[$stage] = [ordered]@{ session = $process.SessionId; normalClose = $false }
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(75)
        do {
            Start-Sleep -Milliseconds 350
            $process.Refresh()
            if ($process.HasExited) { throw "Application exited before readiness ($($process.ExitCode))." }
            $webview = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($process.Id) AND Name = 'msedgewebview2.exe'")
            $entry.readiness = [ordered]@{
                databaseExists = Test-Path -LiteralPath $database -PathType Leaf
                windowHandlePresent = $process.MainWindowHandle -ne [IntPtr]::Zero
                webView2ChildCount = $webview.Count
            }
            $ready = $entry.readiness.databaseExists -and $entry.readiness.windowHandlePresent -and
                $entry.readiness.webView2ChildCount -gt 0
            if (-not $ready -and [DateTime]::UtcNow -ge $deadline) {
                throw ('First launch readiness timed out. ' + ($entry.readiness | ConvertTo-Json -Compress))
            }
        } until ($ready)
        $entry.windowCreated = $true
        $entry.webView2Started = $true
        $entry.database = Test-Database 'check'
        if ($ExerciseRecovery -and $stage -eq 'reopen1') {
            $runtime = Get-Item -LiteralPath @($webview)[0].ExecutablePath
            $signature = Get-AuthenticodeSignature -LiteralPath $runtime.FullName
            $entry.webView2Version = $runtime.VersionInfo.ProductVersion
            $entry.webView2Signature = $signature.Status.ToString()
            $entry.webView2Publisher = $signature.SignerCertificate.Subject
            $entry.modules = @(foreach ($module in $process.Modules) {
                $origin = if ($module.FileName.StartsWith("$env:SystemRoot\", [StringComparison]::OrdinalIgnoreCase)) {
                    'Windows'
                } elseif ($module.FileName.StartsWith("$installDirectory\", [StringComparison]::OrdinalIgnoreCase)) {
                    'Application'
                } elseif ($module.FileName.StartsWith("$($runtime.DirectoryName)\", [StringComparison]::OrdinalIgnoreCase)) {
                    'WebView2'
                } else {
                    'Other'
                }
                [ordered]@{ name = $module.ModuleName; origin = $origin }
            })
        }
        if ($Terminate) {
            $process.Kill()
            if (-not $process.WaitForExit(20000)) { throw 'The owned test process did not terminate.' }
            $entry.forcedTermination = $true
            return
        }
        if (-not $process.CloseMainWindow() -or -not $process.WaitForExit(20000)) {
            throw 'The application did not close normally.'
        }
        $entry.exitCode = $process.ExitCode
        if ($process.ExitCode) { throw 'Application returned a nonzero exit code.' }
        $entry.normalClose = $true
    } finally {
        if (-not $process.HasExited) {
            $process.Kill()
            $null = $process.WaitForExit(10000)
        }
        $process.Dispose()
    }
}

try {
    $report.stage = 'verifyInstallers'
    [string[]]$versions = if ($Scenario -eq 'Upgrade') { '0.1.1','0.1.2' } else { '0.1.2' }
    foreach ($version in $versions) {
        $setup = Join-Path $Installers "SkillVerse AI_${version}_x64-setup.exe"
        $report.installers[$version] = & (Join-Path $PSScriptRoot 'phase19-verify-installer.ps1') -Path $setup -Version $version
    }
    $sentinel = Join-Path $evidenceRoot 'outside-install.txt'
    Set-Content -LiteralPath $sentinel -Value 'Synthetic external file' -Encoding ascii
    $sentinelHash = (Get-FileHash -LiteralPath $sentinel).Hash
    Install-Candidate $versions[0] 'initialInstall'
    Start-And-Close 'firstLaunch'
    $baseline = Join-Path $evidenceRoot 'synthetic-baseline.json'
    $report.stage = 'seedSyntheticData'
    Test-Database 'seed' | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $baseline
    if ($Scenario -eq 'Upgrade') {
        Install-Candidate '0.1.2' 'upgrade'
        Start-And-Close 'upgradeLaunch'
        $report.steps.upgradeLaunch.retention = Test-Database 'compare' $baseline
    }
    $report.stage = 'uninstall'
    $process = Start-Process -FilePath (Join-Path $installDirectory 'uninstall.exe') -ArgumentList '/S' -WindowStyle Hidden -Wait -PassThru
    $report.steps.uninstall = [ordered]@{ exitCode = $process.ExitCode }
    if ($process.ExitCode -or (Test-Path $uninstallKey) -or (Test-Path $installDirectory)) {
        throw 'Uninstall did not remove the registered application files.'
    }
    $report.steps.uninstall.data = Test-Database 'compare' $baseline
    Install-Candidate '0.1.2' 'reinstall'
    if ($ExerciseRecovery) {
        Start-And-Close 'forcedExit' -Terminate
    }
    foreach ($pass in 1..2) {
        Start-And-Close "reopen$pass"
        $report.steps["reopen$pass"].retention = Test-Database 'compare' $baseline
    }
    if ((Get-FileHash -LiteralPath $sentinel).Hash -ne $sentinelHash -or (Test-Path $codexDirectory)) {
        throw 'Unexpected change to the external fixture or Codex directory.'
    }
    $report.externalFixturePreserved = $true
    $report.codexDirectoryCreated = $false
    $report.temporaryFilesRemaining = @(Get-ChildItem -LiteralPath $temp -File -Recurse -Force).Count
    $report.result = 'PASS'
    $report.stage = 'complete'
} finally {
    $env:TEMP = $previousTemp
    $env:TMP = $previousTmp
    $report | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $evidenceRoot 'lifecycle.json')
}
