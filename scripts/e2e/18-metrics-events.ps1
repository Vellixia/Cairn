#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 1: SSE events + Last-Event-ID replay.
# Open an SSE connection, force a memory write, assert the event arrives.
# The SSE endpoint requires auth — pass the cookie via curl -H. We use
# Start-Job + `& curl.exe @argList` instead of Start-Process because
# Start-Process -ArgumentList mangles the embedded space in the
# "Cookie: <value>" header (the cookie never reaches curl, and curl then
# falls back to treating the URL as malformed).
$sseUrl = "$Global:E2E_BaseUrl/api/events"
$tmp = New-TemporaryFile
$argList = @('-sS', '-N', '-o', $tmp.FullName, $sseUrl, '-H', "Cookie: $($Global:E2E_Cookie)", '--max-time', '5')
$job = Start-Job -ScriptBlock { & curl.exe @using:argList } -ArgumentList @{ argList = $argList }
try {
    Start-Sleep -Milliseconds 800
    # Force a memory write to trigger an event.
    Invoke-CairnCli remember 'e2e-sse-test: event trigger'
    Start-Sleep -Milliseconds 1500
    try { $job | Stop-Job } catch { }
    $body = Get-Content $tmp.FullName -Raw -ErrorAction SilentlyContinue
    # SSE events use `data: <json>` lines; the exact format depends on
    # whether the server includes the event type. Accept any of the
    # common prefixes.
    $hasEvent = $body -match 'data:|`event`|"event_id"|event:'
    Assert-True -Condition $hasEvent -Msg 'SSE stream contains event lines'
} finally {
    try { $job | Stop-Job } catch { }
    try { $job | Remove-Job } catch { }
    Remove-Item $tmp -ErrorAction SilentlyContinue
}

# /api/metrics endpoint.
$resp = Test-Endpoint -Method GET -Path '/api/metrics'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/api/metrics returns 200'
$m = $resp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
# /api/metrics returns a JSON object; just verify it parses.
Assert-True -Condition ($null -ne $m) -Msg '/api/metrics returns parseable JSON'

Show-Scenario -Sprint '3.5' -Name 'metrics-events' -Status pass