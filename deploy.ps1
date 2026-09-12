# Build the release binary and put it where the skill and PATH expect it.
#
#   .\deploy.ps1              build, then deploy
#   .\deploy.ps1 -NoBuild     deploy whatever is already in target\release
#
# The skill directory holds the one real copy: it is what the MCP configs point
# at, and it is what travels with the skill. `~\.local\bin` gets hard links to it
# rather than copies, so the bare command name on PATH can never drift from the
# skill's binary. (A hard link needs both paths on the same volume; on anything
# else this falls back to a copy.)
param(
    [string]$SkillDir = "$env:USERPROFILE\.agents\skills\everything-search",
    [string]$BinDir = "$env:USERPROFILE\.local\bin",
    [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$src = Join-Path $root 'target\release\everything-search-mcp.exe'

# A running MCP server holds its own image open, and DSH respawns it within a
# second of being killed - so deleting it usually loses the race. Renaming a
# running executable is allowed on Windows (the mapping follows the file object),
# so the name is freed by moving the old binary aside instead.
function Clear-Target([string]$Path) {
    if (-not (Test-Path $Path)) { return }
    $old = "$Path.old"
    Remove-Item $old -Force -ErrorAction SilentlyContinue
    try { Move-Item $Path $old -Force -ErrorAction Stop }
    catch { Remove-Item $Path -Force }
}

if (-not $NoBuild) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        $env:PATH = 'D:\Rust\1.98.1\bin;' + $env:PATH
    }
    Push-Location $root
    try { cargo build --release } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with $LASTEXITCODE" }
}
if (-not (Test-Path $src)) { throw "no binary at $src - run without -NoBuild first" }

$skillExe = Join-Path $SkillDir 'bin\everything-search-mcp.exe'
New-Item -ItemType Directory -Force -Path (Split-Path $skillExe) | Out-Null
Clear-Target $skillExe
Copy-Item $src $skillExe -Force
Write-Host "skill   $skillExe"

if (Test-Path $BinDir) {
    foreach ($name in 'everything-search-mcp.exe', 'everything-mcp.exe') {
        $dst = Join-Path $BinDir $name
        Clear-Target $dst
        try {
            New-Item -ItemType HardLink -Path $dst -Target $skillExe -ErrorAction Stop | Out-Null
            Write-Host "link    $dst"
        } catch {
            Copy-Item $skillExe $dst -Force
            Write-Host "copy    $dst   (hard link unavailable: $($_.Exception.Message))"
        }
    }
}

# The renamed-aside binaries are still mapped by whatever is running from them,
# so this only succeeds once that process is gone.
foreach ($dir in @((Split-Path $skillExe), $BinDir)) {
    Get-ChildItem "$dir\everything*.exe.old" -ErrorAction SilentlyContinue |
        Remove-Item -Force -ErrorAction SilentlyContinue
}

Write-Host ""
& $skillExe --version
& $skillExe config --target json
