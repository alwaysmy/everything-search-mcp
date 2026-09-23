#Requires -Version 7
<#
  compare-single-backend.ps1
  ==========================
  单后端回归对比：把当前构建与一个 git 基线构建跑同一批 CLI 调用，逐字段比对，
  用来证明「不传 url / 只传 local 时，行为与改造前一致」。

  这是本次多后端改造的验收证据，也可以在任何后续改动后重跑。

  准备基线 exe（从某个 commit 编译）：
    git worktree add $env:TEMP\evmcp-baseline HEAD
    cd $env:TEMP\evmcp-baseline; cargo build --release

  运行：
    pwsh -File .\TEST_SCRIPTS\compare-single-backend.ps1 `
         -BaselineExe $env:TEMP\evmcp-baseline\target\release\everything-search-mcp.exe

  比对规则：
    - 文本输出逐字节比较。
    - JSON 输出只剔除 `elapsed_ms`（每次都不同，不是行为）。**其余字段一律必须一致** ——
      单后端时不允许新增任何字段：客户端的 outputSchema 带 `additionalProperties: false`，
      多一个未声明的字段会让它拒掉**整个响应**。
  退出码：0 = 全部一致；1 = 有差异（差异详情打印在 stdout）。
#>
param(
  [Parameter(Mandatory = $true)][string]$BaselineExe,
  [string]$NewExe = (Join-Path $PSScriptRoot '..\target\release\everything-search-mcp.exe'),
  # 指向一个独立的注册表文件，避免碰到用户真实的 %APPDATA% 配置。
  [string]$ServersFile = (Join-Path $env:TEMP 'evmcp-compare\servers.json'),
  # 查询用的目录。默认取本脚本所在的仓库，所以脚本里不写死任何机器专属路径。
  # 传 -OtherPath 指向一个内容不同的目录（例如放了不少 PDF 的地方）可以多覆盖几个分支。
  [string]$ProjectPath = '',
  [string]$OtherPath = ''
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8

if (-not (Test-Path $BaselineExe)) { throw "baseline exe not found: $BaselineExe" }
if (-not (Test-Path $NewExe)) { throw "new exe not found: $NewExe" }
$env:EVERYTHING_SERVERS_FILE = $ServersFile

# The only field excluded: a wall-clock measurement, different on every run.
$newByDesign = 'elapsed_ms'

function Get-Normalized($value) {
  if ($null -eq $value) { return $null }
  if ($value -is [System.Management.Automation.PSCustomObject]) {
    $o = [ordered]@{}
    foreach ($p in $value.PSObject.Properties) {
      if ($p.Name -in $newByDesign) { continue }
      $o[$p.Name] = Get-Normalized $p.Value
    }
    return [PSCustomObject]$o
  }
  if ($value -is [System.Collections.IEnumerable] -and $value -isnot [string]) {
    # `,` keeps a one-element array an array: PowerShell otherwise unwraps it, and a
    # scalar compared against an array reports a difference that is not there.
    return , @($value | ForEach-Object { Get-Normalized $_ })
  }
  return $value
}

$script:passed = 0
$script:failed = 0

function Test-Same {
  param([string]$Label, [string[]]$OldArgs, [string[]]$NewArgs, [switch]$Json)
  if ($Json) {
    $a = Get-Normalized ((& $BaselineExe @OldArgs | ConvertFrom-Json))
    $b = Get-Normalized ((& $NewExe @NewArgs | ConvertFrom-Json))
    $left = $a | ConvertTo-Json -Depth 30 -Compress
    $right = $b | ConvertTo-Json -Depth 30 -Compress
  }
  else {
    $left = (& $BaselineExe @OldArgs 2>&1 | Out-String)
    $right = (& $NewExe @NewArgs 2>&1 | Out-String)
  }
  if ($left -ceq $right) {
    Write-Host "  [same] $Label"
    $script:passed++
  }
  else {
    Write-Host "  [DIFF] $Label"
    Write-Host "    baseline: $left"
    Write-Host "    current : $right"
    $script:failed++
  }
}

# Fixed paths only: a query over C:\Windows picks up files that change while the
# comparison runs, which shows up as a diff that has nothing to do with the code.
# Both default to this repository, so nothing machine-specific is baked into the file.
if (-not $ProjectPath) { $ProjectPath = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path }
if (-not $OtherPath) { $OtherPath = $ProjectPath }
$P = $ProjectPath
$C = $OtherPath

Write-Host "baseline: $BaselineExe"
Write-Host "current : $NewExe"
Write-Host "registry: $ServersFile`n"

# ---- everything_search -------------------------------------------------------
Test-Same 'search text'        @('search', '*.pdf', '--max', '2', '--path', $C) @('search', '*.pdf', '--max', '2', '--path', $C, '--url', 'local')
Test-Same 'search empty'       @('search', 'zzz-no-such-thing-xyz') @('search', 'zzz-no-such-thing-xyz', '--url', 'local')
Test-Same 'search json'        @('search', '*.pdf', '--max', '3', '--path', $C, '--json') @('search', '*.pdf', '--max', '3', '--path', $C, '--json', '--url', 'local') -Json
Test-Same 'search total'       @('search', '*.pdf', '--max', '2', '--path', $C, '--total', '--json') @('search', '*.pdf', '--max', '2', '--path', $C, '--total', '--json', '--url', 'local') -Json
Test-Same 'search regex'       @('search', '^main\.rs$', '--regex', '--max', '3', '--path', $P, '--json') @('search', '^main\.rs$', '--regex', '--max', '3', '--path', $P, '--json', '--url', 'local') -Json
Test-Same 'search regex typo'  @('search', '^(unclosed[(') @('search', '^(unclosed[(', '--url', 'local')
Test-Same 'search probe'       @('search', 'everything-mcp', '--probe', '--max', '3', '--path', $P, '--json') @('search', 'everything-mcp', '--probe', '--max', '3', '--path', $P, '--json', '--url', 'local') -Json
Test-Same 'search diversify'   @('search', '*.rs', '--sort', 'size', '--per-parent', '2', '--max', '5', '--path', $P, '--json') @('search', '*.rs', '--sort', 'size', '--per-parent', '2', '--max', '5', '--path', $P, '--json', '--url', 'local') -Json
Test-Same 'search folder only' @('search', '', '--type', 'folder', '--path', $P, '--max', '3', '--json') @('search', '', '--type', 'folder', '--path', $P, '--max', '3', '--json', '--url', 'local') -Json

# ---- everything_find_recent --------------------------------------------------
Test-Same 'recent'             @('recent', '--period', '1year', '--max', '3', '--path', $P, '--json') @('recent', '--period', '1year', '--max', '3', '--path', $P, '--json', '--url', 'local') -Json
Test-Same 'recent no expand'   @('recent', '--period', '1min', '--no-expand', '--path', $P, '--json') @('recent', '--period', '1min', '--no-expand', '--path', $P, '--json', '--url', 'local') -Json
Test-Same 'recent numeric'     @('recent', '--period', '90days', '--max', '3', '--path', $P, '--json') @('recent', '--period', '90days', '--max', '3', '--path', $P, '--json', '--url', 'local') -Json
Test-Same 'recent bad period'  @('recent', '--period', 'nonsense') @('recent', '--period', 'nonsense', '--url', 'local')

# ---- everything_count_stats --------------------------------------------------
Test-Same 'count'              @('count', '*.pdf', '--path', $C) @('count', '*.pdf', '--path', $C, '--url', 'local')
Test-Same 'count size cover'   @('count', '*.pdf', '--path', $C, '--exact-size') @('count', '*.pdf', '--path', $C, '--exact-size', '--url', 'local')
# *.rs has more than the 500-row sample, so this is the branch that really pages.
Test-Same 'count size paged'   @('count', '*.rs', '--exact-size', '--json') @('count', '*.rs', '--exact-size', '--json', '--url', 'local') -Json
Test-Same 'count size sample'  @('count', '*.rs', '--json') @('count', '*.rs', '--json', '--url', 'local') -Json
Test-Same 'count breakdown'    @('count', '*.rs', '--path', $P, '--breakdown', '--json') @('count', '*.rs', '--path', $P, '--breakdown', '--json', '--url', 'local') -Json
Test-Same 'count bad sort'     @('count', '*.rs', '--breakdown', '--sort', 'name') @('count', '*.rs', '--breakdown', '--sort', 'name', '--url', 'local')

# ---- everything_file_details -------------------------------------------------
Test-Same 'details dir'        @('details', "$P\3_WorkTools\everything-mcp", '--json') @('details', "$P\3_WorkTools\everything-mcp", '--json', '--url', 'local') -Json
Test-Same 'details file'       @('details', "$P\3_WorkTools\everything-mcp\Cargo.toml", '--preview', '5', '--json') @('details', "$P\3_WorkTools\everything-mcp\Cargo.toml", '--preview', '5', '--json', '--url', 'local') -Json
Test-Same 'details missing'    @('details', 'D:\nope\nothing.txt', '--json') @('details', 'D:\nope\nothing.txt', '--json', '--url', 'local') -Json
Test-Same 'details relative'   @('details', 'relative\path.txt', '--json') @('details', 'relative\path.txt', '--json', '--url', 'local') -Json

Write-Host "`n$script:passed identical, $script:failed different"
exit $(if ($script:failed -eq 0) { 0 } else { 1 })
