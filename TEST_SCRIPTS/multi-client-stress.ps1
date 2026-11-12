# Several independent clients hitting the MCP endpoint at once.
#
# The point is not throughput. It is that concurrent callers must each get *their
# own* answer: the endpoint is stateless and builds a server per request, so a
# response landing on the wrong connection, a shared cursor, or a leaked query
# would show up here as a foreign row or a wildly different total.
#
# Two different things are measured, and only one of them is a failure:
#
#   mismatch  a row that does not belong to this client's query, or a total so far
#             from the baseline that the answer cannot be this client's. A bug.
#   drift     the total moved a little because files changed while the test ran.
#             Everything indexes in real time, so building the project under test
#             legitimately moves its own counts. Reported, never fatal.
#
#   .\TEST_SCRIPTS\multi-client-stress.ps1
#   .\TEST_SCRIPTS\multi-client-stress.ps1 -Clients 8 -Rounds 15
#
# Needs Windows, Everything, and the plugin. Not part of CI.
param(
    [int]$Port = 8286,
    [int]$Clients = 6,
    [int]$Rounds = 5,
    [string]$SearchPath = 'D:\WorkDesigns',
    # A total may move this much before it stops being "the same answer".
    [double]$DriftTolerance = 0.05
)

$ErrorActionPreference = 'Stop'
$endpoint = "http://127.0.0.1:$Port/mcp"

function New-Specs {
    param([string]$Root)
    @(
        @{ Tool = 'everything_search'; Ext = @('md'); Args = @{ query = 'ext:md'; path = $Root; max_results = 3 } }
        @{ Tool = 'everything_search'; Ext = @('dll'); Args = @{ query = '*.dll'; path = $Root; max_results = 3 } }
        @{ Tool = 'everything_search'; Ext = @('png'); Args = @{ query = 'ext:png'; path = $Root; max_results = 3 } }
        @{ Tool = 'everything_count_stats'; Ext = @(); Args = @{ query = 'ext:cs'; path = $Root } }
        @{ Tool = 'everything_search'; Ext = @('pdf'); Args = @{ query = 'ext:pdf'; path = $Root; max_results = 3 } }
        @{
            Tool = 'everything_search_batch'
            Ext  = @('json', 'log', 'py')
            Args = @{
                queries = @(
                    @{ query = 'ext:json'; path = $Root; max_results = 2 },
                    @{ query = 'ext:log'; path = $Root; max_results = 2 },
                    @{ query = 'ext:py'; path = $Root; max_results = 2 }
                )
            }
        }
    )
}

function Invoke-Tool {
    param([string]$Tool, [hashtable]$Arguments)
    $payload = @{
        jsonrpc = '2.0'; id = 1; method = 'tools/call'
        params  = @{ name = $Tool; arguments = $Arguments }
    } | ConvertTo-Json -Depth 10
    # -UseBasicParsing: on Windows PowerShell the default IE path throws a bare
    # NullReferenceException on a request that already succeeded.
    $response = Invoke-WebRequest -Uri $endpoint -Method Post `
        -Headers @{ Accept = 'application/json, text/event-stream' } `
        -ContentType 'application/json' -Body $payload -TimeoutSec 120 -UseBasicParsing
    $parsed = $response.Content | ConvertFrom-Json
    if ($null -eq $parsed.result) { throw "no result: $($response.Content)" }
    if ($parsed.result.isError) { throw "$Tool failed: $($parsed.result.content[0].text)" }
    $parsed.result.structuredContent
}

# The totals a client must see: one number per answer inside the response.
function Get-Totals {
    param([string]$Tool, $Structured)
    switch ($Tool) {
        'everything_count_stats' { return @([int]$Structured.count) }
        'everything_search_batch' {
            return @($Structured.results | ForEach-Object { [int]$_.total })
        }
        default { return @([int]$Structured.total) }
    }
}

# Every row a client got back must match the extension that client asked for.
# A response that belonged to somebody else fails here immediately.
function Test-Rows {
    param([string]$Tool, $Structured, [string[]]$Ext)
    if ($Ext.Count -eq 0) { return @() }
    $groups = if ($Tool -eq 'everything_search_batch') { $Structured.results } else { @($Structured) }
    $bad = @()
    for ($i = 0; $i -lt $groups.Count; $i++) {
        $expected = if ($Ext.Count -eq 1) { $Ext[0] } else { $Ext[$i] }
        foreach ($row in $groups[$i].results) {
            if (-not $row.name.EndsWith(".$expected", [StringComparison]::OrdinalIgnoreCase)) {
                $bad += "expected .$expected, got $($row.name)"
            }
        }
    }
    return $bad
}

if (-not (Test-Path $SearchPath)) { throw "search path '$SearchPath' does not exist" }

$specs = New-Specs -Root $SearchPath

Write-Host "endpoint $endpoint"
Write-Host "clients  $Clients   rounds $Rounds   path $SearchPath"
Write-Host ''

# Solo baseline: the totals each client's answer must be close to.
$baselines = @()
for ($i = 0; $i -lt $Clients; $i++) {
    $spec = $specs[$i % $specs.Count]
    $structured = Invoke-Tool -Tool $spec.Tool -Arguments $spec.Args
    $baselines += , (Get-Totals -Tool $spec.Tool -Structured $structured)
    Write-Host ("baseline[{0}] {1,-26} {2}" -f $i, $spec.Tool, ($baselines[$i] -join '/'))
}
Write-Host ''

$jobs = for ($i = 0; $i -lt $Clients; $i++) {
    $spec = $specs[$i % $specs.Count]
    $baseline = $baselines[$i]
    Start-ThreadJob -ScriptBlock {
        param($endpoint, $spec, $rounds, $baseline, $tolerance)
        $latencies = New-Object 'System.Collections.Generic.List[int]'
        $failures = New-Object 'System.Collections.Generic.List[string]'
        $mismatches = New-Object 'System.Collections.Generic.List[string]'
        $drifts = New-Object 'System.Collections.Generic.List[string]'

        function TotalsOf {
            param($structured)
            switch ($spec.Tool) {
                'everything_count_stats' { return @([int]$structured.count) }
                'everything_search_batch' { return @($structured.results | ForEach-Object { [int]$_.total }) }
                default { return @([int]$structured.total) }
            }
        }

        for ($r = 1; $r -le $rounds; $r++) {
            $body = @{
                jsonrpc = '2.0'; id = $r; method = 'tools/call'
                params  = @{ name = $spec.Tool; arguments = $spec.Args }
            } | ConvertTo-Json -Depth 10
            $sw = [Diagnostics.Stopwatch]::StartNew()
            try {
                $response = Invoke-WebRequest -Uri $endpoint -Method Post `
                    -Headers @{ Accept = 'application/json, text/event-stream' } `
                    -ContentType 'application/json' -Body $body -TimeoutSec 120 -UseBasicParsing
                $sw.Stop()
                $latencies.Add([int]$sw.ElapsedMilliseconds)

                $parsed = $response.Content | ConvertFrom-Json
                if ($parsed.id -ne $r) { $mismatches.Add("id echo: sent $r got $($parsed.id)"); continue }
                if ($parsed.result.isError) { $failures.Add($parsed.result.content[0].text); continue }
                $structured = $parsed.result.structuredContent

                # Rows must belong to this client's query.
                $groups = if ($spec.Tool -eq 'everything_search_batch') { $structured.results } else { @($structured) }
                if ($spec.Ext.Count -gt 0) {
                    for ($g = 0; $g -lt $groups.Count; $g++) {
                        $expected = if ($spec.Ext.Count -eq 1) { $spec.Ext[0] } else { $spec.Ext[$g] }
                        foreach ($row in $groups[$g].results) {
                            if (-not $row.name.EndsWith(".$expected", [StringComparison]::OrdinalIgnoreCase)) {
                                $mismatches.Add("expected .$expected, got $($row.name)")
                            }
                        }
                    }
                }

                # Totals may drift; they may not be somebody else's answer.
                $now = TotalsOf $structured
                for ($k = 0; $k -lt $now.Count; $k++) {
                    $was = [int]$baseline[$k]
                    $delta = [Math]::Abs($now[$k] - $was)
                    if ($was -eq 0) { continue }
                    if ($delta -gt [Math]::Max(25, $was * $tolerance)) {
                        $mismatches.Add("total $k moved $was -> $($now[$k]) (beyond drift)")
                    } elseif ($delta -gt 0) {
                        $drifts.Add("$was->$($now[$k])")
                    }
                }
            } catch {
                $sw.Stop()
                $failures.Add($_.Exception.Message)
            }
        }
        [pscustomobject]@{
            Latencies  = $latencies.ToArray()
            Failures   = $failures.ToArray()
            Mismatches = $mismatches.ToArray()
            Drifts     = $drifts.ToArray()
        }
    } -ArgumentList $endpoint, $spec, $Rounds, $baseline, $DriftTolerance
}

$wall = [Diagnostics.Stopwatch]::StartNew()
$results = $jobs | Wait-Job | Receive-Job
$jobs | Remove-Job -Force
$wall.Stop()

$all = New-Object 'System.Collections.Generic.List[int]'
$failures = New-Object 'System.Collections.Generic.List[string]'
$mismatches = New-Object 'System.Collections.Generic.List[string]'
$drifts = New-Object 'System.Collections.Generic.List[string]'
foreach ($r in $results) {
    foreach ($l in $r.Latencies) { $all.Add($l) }
    foreach ($f in $r.Failures) { $failures.Add($f) }
    foreach ($m in $r.Mismatches) { $mismatches.Add($m) }
    foreach ($d in $r.Drifts) { $drifts.Add($d) }
}

$sorted = $all | Sort-Object
function Pct($values, $p) {
    if ($values.Count -eq 0) { return 0 }
    return $values[[Math]::Min($values.Count - 1, [Math]::Floor($values.Count * $p))]
}

$calls = $all.Count
$perSecond = if ($wall.Elapsed.TotalSeconds -gt 0) { [Math]::Round($calls / $wall.Elapsed.TotalSeconds, 1) } else { 0 }

Write-Host ''
Write-Host ("calls     {0} in {1:N2} s ({2}/s)" -f $calls, $wall.Elapsed.TotalSeconds, $perSecond)
Write-Host ("latency   p50 {0} ms   p95 {1} ms   max {2} ms" -f (Pct $sorted 0.50), (Pct $sorted 0.95), (Pct $sorted 1.0))
Write-Host ("failures  {0}" -f $failures.Count)
Write-Host ("mismatch  {0}" -f $mismatches.Count)
Write-Host ("drift     {0} answers whose total moved (index changed under us)" -f $drifts.Count)
Write-Host ''

foreach ($f in ($failures | Select-Object -First 5)) { Write-Host "  failure: $f" -ForegroundColor Red }
foreach ($m in ($mismatches | Select-Object -First 5)) { Write-Host "  mismatch: $m" -ForegroundColor Red }

if ($failures.Count -gt 0 -or $mismatches.Count -gt 0) {
    Write-Host 'FAIL  concurrent clients did not each get their own answer' -ForegroundColor Red
    exit 1
}
Write-Host "PASS  $calls concurrent calls, every answer belonged to its own client" -ForegroundColor Green
exit 0
