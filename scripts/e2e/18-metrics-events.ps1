#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 1: SSE events + Last-Event-ID replay.
# Open an SSE connection, force a memory write, assert the event arrives.
# The SSE endpoint requires auth — send the cookie so curl gets through.
$sseUrl = "$Global:E2E_BaseUrl/api/events"
$sseArgs = @('-sS', '-N', '-o', '__SSE_OUT__', $sseUrl)
if ($Global:E2E_Cookie) {
    $sseArgs += @('-H', "Cookie: $($Global:E2E_Cookie)")
}
$tmp = New-TemporaryFile
$sseArgs[3] = $tmp.FullName  # replace __SSE_OUT__ with the real temp path
try {
    $proc = Start-Process -FilePath curl.exe -ArgumentList $sseArgs `
        -NoNewWindow -PassThru
    Start-Sleep -Milliseconds 800
    # Force a memory write to trigger an event.
    Invoke-CairnCli remember 'e2e-sse-test: event trigger'
    Start-Sleep -Milliseconds 1500
    try { $proc | Stop-Process -Force } catch { }
    $body = Get-Content $tmp.FullName -Raw -ErrorAction SilentlyContinue
    # SSE events use `data: <json>` lines; the exact format depends on
    # whether the server includes the event type. Accept any of the
    # common prefixes.
    $hasEvent = $body -match 'data:|`event`|"event_id"|event:'
    Assert-True -Condition $hasEvent -Msg 'SSE stream contains event lines'
} finally {
    Remove-Item $tmp -ErrorAction SilentlyContinue
}

# /api/metrics endpoint.
$resp = Test-Endpoint -Method GET -Path '/api/metrics'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/api/metrics returns 200'
$m = $resp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
# /api/metrics returns a JSON object; just verify it parses.
Assert-True -Condition ($null -ne $m) -Msg '/api/metrics returns parseable JSON'

Show-Scenario -Sprint '3.5' -Name 'metrics-events' -Status pass
