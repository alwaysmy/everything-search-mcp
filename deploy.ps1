# Build the plugin DLL, install it into Everything, and sync the skill docs.
#
#   .\deploy.ps1              build, install, sync
#   .\deploy.ps1 -NoBuild     install whatever is already in target\release
#
# v3 has no executable: the endpoint lives inside Everything, so installing the
# DLL *is* the deployment. Needs an elevated PowerShell (the AllowedPlugins write
# is under HKLM).
param(
    [switch]$NoBuild,
    [string]$SkillDir = "$env:USERPROFILE\.agents\skills\everything-search"
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$dll = Join-Path $root 'target\release\everything_mcp_core.dll'

if (-not $NoBuild) {
    Push-Location $root
    try { cargo build --release } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with $LASTEXITCODE" }
}
if (-not (Test-Path $dll)) { throw "no plugin DLL at $dll - run without -NoBuild first" }

Write-Host "plugin  $dll"
& (Join-Path $root 'install-plugin.ps1') -NoBuild
if ($LASTEXITCODE -ne 0) { throw "install-plugin.ps1 failed with $LASTEXITCODE" }

foreach ($f in 'SKILL.md', 'REFERENCE.md', 'INSTALL.md') {
    $src = Join-Path $root "skills\everything-search\$f"
    if (Test-Path $SkillDir) {
        Copy-Item $src (Join-Path $SkillDir $f) -Force
        Write-Host "skill   $(Join-Path $SkillDir $f)"
    }
}

# Nothing to verify from the outside: install-plugin.ps1 already speaks MCP to the
# endpoint and prints the tool count. A stale exe from v2 may still sit in the
# skill's bin/ and on PATH; v3 does not use it, and deleting it here would break a
# stdio config that still points at it.
