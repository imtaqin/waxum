# wa-rs Windows installer / updater.
#
# Fetches the latest release binary from
# https://github.com/fdciabdul/wa-rs/releases, installs it into
# C:\ProgramData\wa-rs, and registers a Windows service (via `sc.exe`) so
# it auto-restarts. First run prompts whether to enable a scheduled task
# for nightly auto-update.
#
# Usage (elevated PowerShell):
#   irm https://raw.githubusercontent.com/fdciabdul/wa-rs/main/scripts/install.ps1 | iex
#   .\install.ps1 install
#   .\install.ps1 update
#   .\install.ps1 uninstall

param(
    [ValidateSet('install','update','uninstall','help')]
    [string]$Command = 'install'
)

$ErrorActionPreference = 'Stop'
$Repo         = 'fdciabdul/wa-rs'
$InstallDir   = "$env:ProgramData\wa-rs"
$BinPath      = Join-Path $InstallDir 'wa-rs.exe'
$EnvFile      = Join-Path $InstallDir '.env'
$VersionFile  = Join-Path $InstallDir '.version'
$SessionsDir  = Join-Path $InstallDir 'whatsapp_sessions'
$ServiceName  = 'wa-rs'
$TaskName     = 'wa-rs-auto-update'

function Show-Banner {
    Write-Host ''
    Write-Host '██╗    ██╗ █████╗       ██████╗ ███████╗' -ForegroundColor Cyan
    Write-Host '██║    ██║██╔══██╗      ██╔══██╗██╔════╝' -ForegroundColor Cyan
    Write-Host '██║ █╗ ██║███████║█████╗██████╔╝███████╗' -ForegroundColor Cyan
    Write-Host '██║███╗██║██╔══██║╚════╝██╔══██╗╚════██║' -ForegroundColor Cyan
    Write-Host '╚███╔███╔╝██║  ██║      ██║  ██║███████║' -ForegroundColor Cyan
    Write-Host ' ╚══╝╚══╝ ╚═╝  ╚═╝      ╚═╝  ╚═╝╚══════╝' -ForegroundColor Cyan
    Write-Host ''
    Write-Host '  WhatsApp Gateway REST API - installer' -ForegroundColor White
    Write-Host '  https://github.com/fdciabdul/wa-rs' -ForegroundColor DarkGray
    Write-Host ''
}

function Log { Write-Host "[wa-rs] $args" -ForegroundColor Cyan }
function Ok  { Write-Host "[wa-rs] $args" -ForegroundColor Green }
function Warn{ Write-Host "[wa-rs] $args" -ForegroundColor Yellow }
function Die { Write-Host "[wa-rs] $args" -ForegroundColor Red; exit 1 }

function Assert-Admin {
    $current = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($current)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        Die 'must run in an elevated PowerShell (Run as Administrator)'
    }
}

function Detect-Arch {
    # Release workflow only ships windows-amd64 today. ARM64 boxes can
    # still install if they run under x64 emulation.
    switch ($env:PROCESSOR_ARCHITECTURE) {
        'AMD64' { return 'windows-amd64' }
        'ARM64' { Warn 'no native windows-arm64 release yet — falling back to windows-amd64 (runs under x64 emulation)'; return 'windows-amd64' }
        default { Die "unsupported architecture: $($env:PROCESSOR_ARCHITECTURE)" }
    }
}

function Get-LatestTag {
    $headers = @{ 'User-Agent' = 'wa-rs-installer' }
    $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers $headers
    if (-not $rel.tag_name) { Die 'could not resolve latest release tag from GitHub' }
    return $rel.tag_name
}

function Read-YesNo($prompt, $default = 'n') {
    $suffix = if ($default -eq 'y') { '[Y/n]' } else { '[y/N]' }
    $ans = Read-Host "? $prompt $suffix"
    if ([string]::IsNullOrWhiteSpace($ans)) { $ans = $default }
    return $ans.ToLower() -eq 'y'
}

function New-RandomHex($bytes) {
    $b = New-Object byte[] $bytes
    [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($b)
    ($b | ForEach-Object { $_.ToString('x2') }) -join ''
}

function Ensure-Dirs {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    New-Item -ItemType Directory -Force -Path $SessionsDir | Out-Null
}

function Download-Binary($tag, $arch) {
    $version = $tag.TrimStart('v')
    $url  = "https://github.com/$Repo/releases/download/$tag/wa-rs-$version-$arch.zip"
    $tmp  = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP "wa-rs-$([guid]::NewGuid().Guid)")
    try {
        Log "downloading $tag ($arch)..."
        $zip = Join-Path $tmp 'wa-rs.zip'
        Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
        Expand-Archive -Path $zip -DestinationPath $tmp -Force
        $binSrc = Get-ChildItem -Path $tmp -Filter 'wa-rs.exe' -Recurse | Select-Object -First 1
        if (-not $binSrc) { Die 'release archive missing wa-rs.exe' }

        # Stop service before overwriting the binary — Windows locks a
        # running exe. Ignore errors if the service isn't installed yet.
        try { Stop-Service -Name $ServiceName -ErrorAction Stop } catch {}

        Copy-Item -Path $binSrc.FullName -Destination $BinPath -Force
        Set-Content -Path $VersionFile -Value $tag
        Ok "installed binary at $BinPath"
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}

function Write-EnvIfAbsent {
    if (Test-Path $EnvFile) {
        Log "env file exists at $EnvFile - leaving as-is"
        return
    }
    $jwt   = New-RandomHex 48
    $admin = New-RandomHex 24
    $lines = @(
        '# wa-rs environment. Edit and then: Restart-Service wa-rs',
        "DATABASE_URL=sqlite://$($InstallDir -replace '\\','/')/wa-rs.db",
        "JWT_SECRET=$jwt",
        "SUPERADMIN_TOKEN=$admin",
        "WHATSAPP_STORAGE_PATH=$($SessionsDir -replace '\\','/')",
        'RUST_LOG=wa_rs=info,tower_http=info',
        'WA_RS_BLOCKING_THREADS=256',
        'NATS_URL='
    )
    Set-Content -Path $EnvFile -Value $lines -Encoding utf8
    Ok "created $EnvFile - review before starting"
}

function Register-Service {
    # Use sc.exe rather than New-Service so we can pass an ExpandString
    # BinaryPathName that quotes the exe path. Environment is loaded from
    # the .env by a small launcher batch we generate below.
    $launcher = Join-Path $InstallDir 'run.cmd'
    $launcherBody = @"
@echo off
cd /d "$InstallDir"
for /f "usebackq eol=# tokens=1,* delims==" %%A in ("$EnvFile") do (
    set "%%A=%%B"
)
"$BinPath"
"@
    Set-Content -Path $launcher -Value $launcherBody -Encoding ascii

    $existing = sc.exe query $ServiceName 2>$null
    if ($LASTEXITCODE -ne 0) {
        sc.exe create $ServiceName binPath= "cmd.exe /c `"$launcher`"" start= auto DisplayName= 'wa-rs WhatsApp Gateway' | Out-Null
        sc.exe description $ServiceName 'WhatsApp Gateway REST API (wa-rs)' | Out-Null
    }
    Ok "registered Windows service '$ServiceName'"
}

function Install-UpdateTask {
    $update = @"
`$ErrorActionPreference = 'Stop'
`$repo    = '$Repo'
`$binPath = '$BinPath'
`$verFile = '$VersionFile'
switch (`$env:PROCESSOR_ARCHITECTURE) {
    'AMD64' { `$arch = 'windows-amd64' }
    'ARM64' { `$arch = 'windows-arm64' }
    default { exit 0 }
}
`$headers = @{ 'User-Agent' = 'wa-rs-installer' }
`$rel = Invoke-RestMethod -Uri "https://api.github.com/repos/`$repo/releases/latest" -Headers `$headers
`$tag = `$rel.tag_name
`$current = if (Test-Path `$verFile) { Get-Content `$verFile } else { '' }
if (`$tag -eq `$current) { exit 0 }
`$version = `$tag.TrimStart('v')
`$url = "https://github.com/`$repo/releases/download/`$tag/wa-rs-`$version-`$arch.zip"
`$tmp = New-Item -ItemType Directory -Force -Path (Join-Path `$env:TEMP "wa-rs-`$([guid]::NewGuid().Guid)")
`$zip = Join-Path `$tmp 'wa-rs.zip'
Invoke-WebRequest -Uri `$url -OutFile `$zip -UseBasicParsing
Expand-Archive -Path `$zip -DestinationPath `$tmp -Force
`$src = Get-ChildItem -Path `$tmp -Filter 'wa-rs.exe' -Recurse | Select-Object -First 1
Stop-Service -Name '$ServiceName' -ErrorAction SilentlyContinue
Copy-Item -Path `$src.FullName -Destination `$binPath -Force
Set-Content -Path `$verFile -Value `$tag
Start-Service -Name '$ServiceName'
Remove-Item -Recurse -Force `$tmp -ErrorAction SilentlyContinue
"@
    $updateScript = Join-Path $InstallDir 'auto-update.ps1'
    Set-Content -Path $updateScript -Value $update -Encoding utf8

    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    $action    = New-ScheduledTaskAction   -Execute 'powershell.exe' -Argument "-NoProfile -WindowStyle Hidden -File `"$updateScript`""
    $trigger   = New-ScheduledTaskTrigger  -Daily -At 3:15am
    $settings  = New-ScheduledTaskSettingsSet -StartWhenAvailable -AllowStartIfOnBatteries
    $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Force | Out-Null
    Ok 'auto-update scheduled task installed (nightly 03:15)'
}

function Remove-UpdateTask {
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
}

function Invoke-Install {
    Show-Banner
    Assert-Admin
    $arch = Detect-Arch
    $tag  = Get-LatestTag
    Log "latest release: $tag"

    Ensure-Dirs
    Download-Binary -tag $tag -arch $arch
    Write-EnvIfAbsent
    Register-Service

    if (Read-YesNo 'enable nightly auto-update task (03:15 daily)?' 'y') {
        Install-UpdateTask
    } else {
        Remove-UpdateTask
        Log "auto-update disabled - update later with: irm https://raw.githubusercontent.com/$Repo/main/scripts/install.ps1 | iex; Invoke-Update"
    }

    if (Read-YesNo 'start wa-rs now?' 'y') {
        Start-Service -Name $ServiceName
        Start-Sleep -Seconds 2
        Get-Service -Name $ServiceName | Format-Table Name, Status, StartType
    } else {
        Warn 'not started - later: Start-Service wa-rs'
    }

    Ok "done. edit $EnvFile then Restart-Service wa-rs after changes."
}

function Invoke-Update {
    Show-Banner
    Assert-Admin
    $arch = Detect-Arch
    $tag  = Get-LatestTag
    $current = if (Test-Path $VersionFile) { Get-Content $VersionFile } else { '' }
    if ($tag -eq $current) {
        Ok "already on $tag"
        return
    }
    Download-Binary -tag $tag -arch $arch
    Start-Service -Name $ServiceName -ErrorAction SilentlyContinue
    Ok "updated $current -> $tag"
}

function Invoke-Uninstall {
    Show-Banner
    Assert-Admin
    if (Read-YesNo "stop and remove wa-rs service? (data in $InstallDir kept)" 'n') {
        Stop-Service -Name $ServiceName -ErrorAction SilentlyContinue
        sc.exe delete $ServiceName | Out-Null
        Remove-UpdateTask
        Remove-Item -Path $BinPath -ErrorAction SilentlyContinue
        Ok "removed service, binary, scheduled task. $InstallDir kept."
        Warn "delete manually if you're done: Remove-Item -Recurse -Force '$InstallDir'"
    } else {
        Log 'aborted'
    }
}

switch ($Command) {
    'install'   { Invoke-Install }
    'update'    { Invoke-Update }
    'uninstall' { Invoke-Uninstall }
    'help'      { Show-Banner; Write-Host 'Usage: install.ps1 [install|update|uninstall]' }
}
