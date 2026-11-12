# Does one slow tool call delay the calls arriving beside it?
#
# This is the live twin of tests/plugin_http.rs::concurrent_requests_do_not_queue_behind_each_other.
# The unit test proves the endpoint builds a server per request (deterministic, runs
# in CI, needs no Everything). This proves the same thing against a real index, which
# is where the original regression showed up as "a 730 ms count_stats makes a 25 ms
# search wait 829 ms".
#
# Run it after installing a build, on a machine with Everything + the plugin:
#
#   .\TEST_SCRIPTS\parallel-probe.ps1
#   .\TEST_SCRIPTS\parallel-probe.ps1 -Port 8286 -SlowPath D:\Projects
#
# Exit code 0 = no head-of-line blocking detected. 1 = a slow call delayed a fast
# one, which means something serialized the tool layer again.
param(
    [int]$Port = 8286,
    # Empty means "the repository root", so the script runs on any checkout
    # without being told where it lives. Point it at a big tree for a heavier read.
    [string]$SlowPath = '',
    [int]$FastCalls = 3,
    # A fast call may take this multiple of its solo latency before we call it
    # blocked. Absolute floor keeps a 1 ms solo measurement from failing on noise.
    [double]$Tolerance = 5.0,
    [int]$FloorMs = 150
)

$ErrorActionPreference = 'Stop'
$endpoint = "http://127.0.0.1:$Port/mcp"

if ([string]::IsNullOrWhiteSpace($SlowPath)) {
    $SlowPath = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
}
if (-not (Test-Path $SlowPath)) { throw "slow-query path '$SlowPath' does not exist" }

function Invoke-Tool {
    param([string]$Name, [hashtable]$Arguments)
    # -UseBasicParsing is mandatory on Windows PowerShell 5.1: without it the
    # default IE path throws a bare NullReferenceException on a working request.
    $body = @{
        jsonrpc = '2.0'; id = 1; method = 'tools/call'
        params  = @{ name = $Name; arguments = $Arguments }
    } | ConvertTo-Json -Depth 8
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $response = Invoke-WebRequest -Uri $endpoint -Method Post `
        -Headers @{ Accept = 'application/json, text/event-stream' } `
        -ContentType 'application/json' -Body $body -TimeoutSec 120 -UseBasicParsing
    $sw.Stop()
    $parsed = $response.Content | ConvertFrom-Json
    if ($parsed.result.isError) {
        throw "$Name failed: $($parsed.result.content[0].text)"
    }
    [pscustomobject]@{ Ms = [int]$sw.ElapsedMilliseconds; Reply = $parsed }
}

$slow = { Invoke-Tool 'everything_count_stats' @{ query = '*'; path = $SlowPath; exact_size = $true } }
$fast = { Invoke-Tool 'everything_search' @{ query = '*.md'; max_results = 1 } }

Write-Host "endpoint  $endpoint"
Write-Host "slow      count_stats exact_size over $SlowPath"
Write-Host ''

$solo = & $slow
$fastSolo = & $fast
Write-Host ("solo      slow {0,5} ms   fast {1,4} ms" -f $solo.Ms, $fastSolo.Ms)

# Hold one slow call open, then measure the fast ones that land beside it.
$job = Start-Job -ScriptBlock {
    param($port, $path)
    $body = @{
        jsonrpc = '2.0'; id = 9; method = 'tools/call'
        params  = @{ name = 'everything_count_stats'; arguments = @{ query = '*'; path = $path; exact_size = $true } }
    } | ConvertTo-Json -Depth 8
    Invoke-WebRequest -Uri "http://127.0.0.1:$port/mcp" -Method Post `
        -Headers @{ Accept = 'application/json, text/event-stream' } `
        -ContentType 'application/json' -Body $body -TimeoutSec 120 -UseBasicParsing | Out-Null
} -ArgumentList $Port, $SlowPath

Start-Sleep -Milliseconds 150
$during = @()
for ($i = 0; $i -lt $FastCalls; $i++) { $during += (& $fast).Ms }

$job | Wait-Job -Timeout 120 | Out-Null
$job | Remove-Job -Force

$worst = ($during | Measure-Object -Maximum).Maximum
$limit = [Math]::Max($FloorMs, [int]($fastSolo.Ms * $Tolerance))
Write-Host ("during    fast {0}" -f (($during | ForEach-Object { "$_ ms" }) -join ' / '))
Write-Host ("limit     {0} ms  (max of {1} ms floor and {2}x solo)" -f $limit, $FloorMs, $Tolerance)
Write-Host ''

if ($worst -gt $limit) {
    Write-Host "FAIL  worst concurrent fast call waited $worst ms - the tool layer is serializing again" -ForegroundColor Red
    exit 1
}
Write-Host "PASS  worst concurrent fast call was $worst ms - calls are independent" -ForegroundColor Green
exit 0
