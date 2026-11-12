# Assemble the two release archives.
#
#   .\release.ps1                    build, then pack both archives into .\dist
#   .\release.ps1 -NoBuild           pack whatever is already in target\release
#   .\release.ps1 -Version 0.3.11    override the version read from Cargo.toml
#
# Two products, because they are installed differently and a user wants one of
# them, not both:
#
#   everything-search-skill-<v>.zip    the stdio MCP server + the agent skill
#   everything-search-plugin-<v>.zip   the in-process Everything plugin + installer
#
# The plugin archive exists because the plugin is a DLL that Everything loads from
# its own directory, and nobody should have to build Rust to get it. PLUGIN.md
# travels with it: an unsigned DLL that is missing from Everything's allowlist
# fails silently, and that is not something a user can be expected to guess.
param(
    [string]$Version = '',
    [string]$OutDir = 'dist',
    [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
Push-Location $root
try {
    if (-not $Version) {
        $manifest = Get-Content (Join-Path $root 'Cargo.toml') -Raw
        if ($manifest -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
            throw 'could not read the version from Cargo.toml'
        }
        $Version = $Matches[1]
    }

    if (-not $NoBuild) {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed with $LASTEXITCODE" }
    }

    $exe = Join-Path $root 'target\release\everything-search-mcp.exe'
    $dll = Join-Path $root 'target\release\everything_mcp_core.dll'
    if (-not (Test-Path $exe)) { throw "no binary at $exe - run without -NoBuild first" }
    if (-not (Test-Path $dll)) { throw "no plugin at $dll - run without -NoBuild first" }

    $out = Join-Path $root $OutDir
    New-Item -ItemType Directory -Force -Path $out | Out-Null

    function Stage([string]$Name) {
        $path = Join-Path $out "stage-$Name"
        if (Test-Path $path) { Remove-Item $path -Recurse -Force }
        New-Item -ItemType Directory -Force -Path $path | Out-Null
        return $path
    }

    function Pack([string]$Stage, [string]$Archive) {
        $target = Join-Path $out $Archive
        if (Test-Path $target) { Remove-Item $target -Force }
        Compress-Archive -Path (Join-Path $Stage '*') -DestinationPath $target -CompressionLevel Optimal
        $size = [Math]::Round((Get-Item $target).Length / 1KB)
        Write-Host ("{0,-40} {1,6} KB" -f $Archive, $size)
    }

    # --- stdio server + skill -------------------------------------------------
    $skill = Stage 'skill'
    Copy-Item $exe (Join-Path $skill 'everything-search-mcp.exe')
    Copy-Item (Join-Path $root 'skills\everything-search\SKILL.md') $skill
    Copy-Item (Join-Path $root 'skills\everything-search\INSTALL.md') $skill
    Copy-Item (Join-Path $root 'README.md') $skill
    Copy-Item (Join-Path $root 'LICENSE') $skill
    Pack $skill "everything-search-skill-$Version.zip"

    # --- in-process plugin ----------------------------------------------------
    $plugin = Stage 'plugin'
    # Everything derives the plugin name from the file name, so the DLL ships
    # under the name the installer and Everything.ini already expect.
    Copy-Item $dll (Join-Path $plugin 'everything_search_mcp64.dll')
    Copy-Item (Join-Path $root 'install-plugin.ps1') $plugin
    Copy-Item (Join-Path $root 'PLUGIN.md') $plugin
    Copy-Item (Join-Path $root 'LICENSE') $plugin
    Pack $plugin "everything-search-plugin-$Version.zip"

    Remove-Item $skill, $plugin -Recurse -Force
    Write-Host ''
    Write-Host "version $Version -> $(Join-Path $root $OutDir)"
}
finally {
    Pop-Location
}
