# Install the in-process MCP plugin into Everything 1.5, without touching its UI.
#
#   .\install-plugin.ps1                build, install, restart Everything, verify
#   .\install-plugin.ps1 -NoBuild       install the DLL already in target\release
#   .\install-plugin.ps1 -NoRestart     register everything, leave the running host alone
#
# The plugin is a DLL that Everything loads into its own process; the MCP endpoint
# then exists exactly as long as Everything does. Registering it is a file and
# registry job, so it works on a machine where nobody can click through Options:
#
#   Plugins\<name>.dll                    the module itself
#   Everything.ini  plugin_filename       the load list (in load order)
#   Everything.ini  plugin_enabled        one flag per entry, same order
#   Plugins.ini     [<name>] enabled=1    per-plugin settings; 1 means start it
#   HKLM\SOFTWARE\voidtools\Everything
#                   AllowedPlugins        third-party DLLs Everything will load;
#                                         a SEMICOLON-separated list, and a bad
#                                         separator silently disables every entry
#                                         in it - that is how an unsigned plugin
#                                         gets refused before its load is even
#                                         logged.
#
# Everything has no reload path for a plugin DLL: it is loaded at startup, so
# installing redirects the host through one exit/start cycle. The restart is
# issued with -startup so the window stays out of the foreground.
#
# Administration is required for the HKLM allowlist write; run this from an
# elevated PowerShell. The Everything process itself does not need to be elevated.
param(
    [string]$EverythingDir = '',
    [string]$PluginName = 'everything_search_mcp64.dll',
    [int]$Port = 8286,
    [switch]$NoBuild,
    [switch]$NoRestart
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$dllName = [System.IO.Path]::GetFileNameWithoutExtension($PluginName)
$source = Join-Path $root 'target\release\everything_mcp_core.dll'
$regPath = 'HKLM:\SOFTWARE\voidtools\Everything'

function Test-Elevated {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    (New-Object Security.Principal.WindowsPrincipal($id)).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-EverythingExe([string]$Hint) {
    if ($Hint) {
        $candidate = if (Test-Path $Hint -PathType Container) {
            Join-Path $Hint 'Everything.exe'
        } else { $Hint }
        if ($candidate -notmatch '\\Everything\.exe$') {
            $candidate = Join-Path (Split-Path $candidate) 'Everything.exe'
        }
        if (Test-Path $candidate) { return (Resolve-Path $candidate).Path }
        throw "no Everything.exe at $candidate"
    }
    $running = Get-Process Everything -ErrorAction SilentlyContinue |
        Where-Object { $_.Path } | Select-Object -First 1
    if ($running) { return $running.Path }
    foreach ($dir in @($env:ProgramFiles, ${env:ProgramFiles(x86)}, 'D:\MyPrograms\1_Tools\Everything')) {
        if (-not $dir) { continue }
        $candidate = Join-Path $dir 'Everything\Everything.exe'
        if (Test-Path $candidate) { return $candidate }
    }
    throw 'cannot find Everything.exe - pass -EverythingDir'
}

# Replace a file that the running host may still have mapped. Windows refuses to
# unlink a loaded image, but renaming it is allowed, so free the name first.
function Clear-Target([string]$Path) {
    if (-not (Test-Path $Path)) { return }
    $old = "$Path.old"
    Remove-Item $old -Force -ErrorAction SilentlyContinue
    try { Move-Item $Path $old -Force -ErrorAction Stop } catch { Remove-Item $Path -Force }
}

# Rewrite one `key=value` line of an ini-style file, keeping every other line.
function Set-IniLine([string]$File, [string]$Key, [string]$Value) {
    $lines = @(Get-Content -LiteralPath $File)
    $pattern = "^$([regex]::Escape($Key))="
    $found = $false
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match $pattern) {
            $lines[$i] = "$Key=$Value"
            $found = $true
            break
        }
    }
    if (-not $found) { $lines += "$Key=$Value" }
    Set-Content -LiteralPath $File -Value $lines -Encoding utf8
}

function Get-IniValue([string]$File, [string]$Key) {
    $line = Select-String -LiteralPath $File -Pattern "^$([regex]::Escape($Key))=" |
        Select-Object -First 1
    if (-not $line) { return '' }
    $line.Line.Substring($Key.Length + 1)
}

# A plugin section in Plugins.ini holds its settings; `enabled=1` is what makes
# Everything start (not merely load) a third-party plugin.
function Enable-PluginSection([string]$File, [string]$Section) {
    if (-not (Test-Path $File)) { Set-Content -LiteralPath $File -Value '[plugins]' -Encoding utf8 }
    $lines = @(Get-Content -LiteralPath $File)
    $header = "[$Section]"
    $start = -1
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i].Trim() -ieq $header) { $start = $i; break }
    }
    if ($start -lt 0) {
        $lines += ''
        $lines += $header
        $lines += 'enabled=1'
    } else {
        $end = $lines.Count
        for ($i = $start + 1; $i -lt $lines.Count; $i++) {
            if ($lines[$i].TrimStart().StartsWith('[')) { $end = $i; break }
        }
        $set = $false
        for ($i = $start + 1; $i -lt $end; $i++) {
            if ($lines[$i] -match '^\s*enabled\s*=') {
                $lines[$i] = "enabled=1"
                $set = $true
                break
            }
        }
        if (-not $set) { $lines = $lines[0..$start] + 'enabled=1' + $lines[($start + 1)..($lines.Count - 1)] }
    }
    Set-Content -LiteralPath $File -Value $lines -Encoding utf8
}

function Add-AllowedPlugin([string]$Name) {
    $key = Get-Item $regPath -ErrorAction SilentlyContinue
    $current = if ($key) { [string]$key.GetValue('AllowedPlugins') } else { '' }
    $allowed = @($current -split ';' | Where-Object { $_.Trim() } | ForEach-Object { $_.Trim() })
    if ($allowed -contains $Name) { return $current }
    $updated = (@($allowed) + $Name) -join ';'
    New-Item -Path $regPath -Force | Out-Null
    Set-ItemProperty -Path $regPath -Name 'AllowedPlugins' -Value $updated -Type String
    $updated
}

function Wait-Port([int]$Port, [int]$TimeoutSeconds = 30) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $client = New-Object System.Net.Sockets.TcpClient
        try {
            $client.Connect('127.0.0.1', $Port)
            if ($client.Connected) { return $true }
        } catch { } finally { $client.Dispose() }
        Start-Sleep -Milliseconds 250
    }
    $false
}

if (-not (Test-Elevated)) {
    throw 'the AllowedPlugins write is under HKLM - start an elevated PowerShell and rerun'
}

$exe = Get-EverythingExe $EverythingDir
$everythingDir = Split-Path $exe
$pluginsDir = Join-Path $everythingDir 'Plugins'
$ini = Join-Path $everythingDir 'Everything.ini'
$pluginIni = Join-Path $everythingDir 'Plugins.ini'
Write-Host "host    $exe"

if (-not $NoBuild) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        $candidates = @($env:CARGO_HOME, (Join-Path $env:USERPROFILE '.cargo')) |
            Where-Object { $_ } | ForEach-Object { Join-Path $_ 'bin' }
        foreach ($bin in $candidates) {
            if (Test-Path (Join-Path $bin 'cargo.exe')) { $env:PATH = "$bin;$env:PATH"; break }
        }
    }
    Push-Location $root
    try { cargo build --release } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with $LASTEXITCODE" }
}
if (-not (Test-Path $source)) { throw "no DLL at $source - run without -NoBuild first" }

# Everything rewrites Everything.ini when it exits, so the host goes down before
# the ini is edited - otherwise the exit would put the old load list back.
if (-not $NoRestart) {
    Get-Process Everything -ErrorAction SilentlyContinue | ForEach-Object { & $exe -exit | Out-Null }
    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-Date) -lt $deadline -and (Get-Process Everything -ErrorAction SilentlyContinue)) {
        Start-Sleep -Milliseconds 300
    }
    $left = @(Get-Process Everything -ErrorAction SilentlyContinue)
    if ($left.Count -gt 0) { throw "Everything did not exit: $($left.Id -join ', ')" }
}

New-Item -ItemType Directory -Force -Path $pluginsDir | Out-Null
$target = Join-Path $pluginsDir $PluginName
Clear-Target $target
Copy-Item $source $target -Force
Write-Host "module  $target"

# Load list: append the plugin name without disturbing the existing order, and keep
# plugin_enabled exactly as long as plugin_filename.
$filenames = @((Get-IniValue $ini 'plugin_filename') -split ',' |
    Where-Object { $_.Trim() } | ForEach-Object { $_.Trim() })
$enabled = @((Get-IniValue $ini 'plugin_enabled') -split ',' |
    Where-Object { $_.Trim() } | ForEach-Object { $_.Trim() })
while ($enabled.Count -lt $filenames.Count) { $enabled += '1' }
$index = -1
for ($i = 0; $i -lt $filenames.Count; $i++) {
    if ($filenames[$i] -ieq $PluginName) { $index = $i; break }
}
if ($index -lt 0) {
    $filenames += $PluginName
    $enabled += '1'
    $index = $filenames.Count - 1
}
if ($enabled[$index] -ne '1') { $enabled[$index] = '1' }
Set-IniLine $ini 'plugin_filename' ($filenames -join ',')
Set-IniLine $ini 'plugin_enabled' ($enabled -join ',')
Write-Host "ini     $ini  (plugin_filename index $index = $PluginName, enabled)"

Enable-PluginSection $pluginIni $PluginName
Write-Host "ini     $pluginIni  [$PluginName] enabled=1"

$allowed = Add-AllowedPlugin $PluginName
Write-Host "policy  AllowedPlugins=$allowed"

if ($NoRestart) {
    Write-Host ''
    Write-Host 'registered; restart Everything to load the plugin (-NoRestart was given)'
    return
}

# Everything only loads plugins at startup, and it starts in the background so a
# scripted restart never takes the foreground.
Start-Process -FilePath $exe -ArgumentList '-startup' -WindowStyle Hidden

$log = Join-Path $env:LOCALAPPDATA 'everything-search-mcp\plugin.log'
New-Item -ItemType Directory -Force -Path (Split-Path $log) | Out-Null
Remove-Item $log -Force -ErrorAction SilentlyContinue
if (-not (Wait-Port $Port 30)) {
    Write-Host ''
    Write-Host "port $Port never opened - inspect $log"
    throw "plugin did not start the endpoint on $Port"
}
Write-Host "endpoint http://127.0.0.1:$Port/"

$body = @{
    jsonrpc = '2.0'; id = 1; method = 'initialize'
    params  = @{
        protocolVersion = '2025-06-18'; capabilities = @{}
        clientInfo      = @{ name = 'install-plugin'; version = '1' }
    }
} | ConvertTo-Json -Depth 6
# -UseBasicParsing is not optional here. On Windows PowerShell 5.1 the default
# path drives the IE engine, and when that engine is unavailable Invoke-WebRequest
# throws a bare NullReferenceException - on a request that actually succeeded.
# Confirmed deterministic: same body, same endpoint, with the switch removed the
# call fails every time. Do not "fix" this with a retry loop; it hides a real bug.
$response = Invoke-WebRequest -Uri "http://127.0.0.1:$Port/mcp" -Method Post `
    -Headers @{ Accept = 'application/json, text/event-stream' } `
    -ContentType 'application/json' -Body $body -TimeoutSec 15 -UseBasicParsing
$server = ($response.Content | ConvertFrom-Json).result.serverInfo
Write-Host "verified $($server.name) $($server.version)"
if (Test-Path $log) { Get-Content $log | ForEach-Object { Write-Host "log     $_" } }
