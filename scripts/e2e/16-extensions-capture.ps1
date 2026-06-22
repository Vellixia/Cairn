#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 21: browser extension capture — /api/extensions/capture loopback-only.
$resp = Test-Endpoint -Method POST -Path '/api/extensions/capture' -Body @{
    kind = 'selection'
    url = 'https://example.com/page'
    title = 'Test Page'
    text = 'selected text for e2e'
    captured_at = '2026-06-21T00:00:00Z'
}
Assert-Eq -Expected 201 -Actual $resp.StatusCode -Msg '/api/extensions/capture returns 201'
$body = $resp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($body.memory_id -ne $null) -Msg 'capture response has a memory_id'
Assert-Contains -Haystack $body.memory_id -Needle '-' -Msg 'capture memory_id is a UUID'

Show-Scenario -Sprint '5.0' -Name 'extensions-capture' -Status pass
