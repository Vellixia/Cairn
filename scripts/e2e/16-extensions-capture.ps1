#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 21: browser extension capture — /api/extensions/capture loopback-only.
# v0.5.0 review-fix #1 tightened is_local_request to require a loopback
# Origin header (pre-fix a missing Origin was silently accepted as a defense-
# in-depth gap; the auth middleware's ConnectInfo check still catches the
# network layer but the handler boundary is now strict). The e2e harness
# uses curl.exe directly so we can pass the Origin header that Test-Endpoint
# doesn't expose.
$body = @{
    kind = 'selection'
    url = 'https://example.com/page'
    title = 'Test Page'
    text = 'selected text for e2e'
    captured_at = '2026-06-21T00:00:00Z'
} | ConvertTo-Json -Compress
$tmp = New-TemporaryFile
try {
    $code = & curl.exe -sS -o $tmp.FullName -w '%{http_code}' `
        -X POST `
        -H 'content-type: application/json' `
        -H "Cookie: $($Global:E2E_Cookie)" `
        -H 'Origin: http://127.0.0.1:7777' `
        -d $body `
        "$Global:E2E_BaseUrl/api/extensions/capture"
    $respBody = Get-Content $tmp.FullName -Raw
    Assert-Eq -Expected 201 -Actual ([int]$code) -Msg '/api/extensions/capture returns 201'
    $parsed = $respBody | ConvertFrom-Json -ErrorAction SilentlyContinue
    Assert-True -Condition ($parsed.memory_id -ne $null) -Msg 'capture response has a memory_id'
    Assert-Contains -Haystack $parsed.memory_id -Needle '-' -Msg 'capture memory_id is a UUID'
} finally {
    Remove-Item $tmp -ErrorAction SilentlyContinue
}

Show-Scenario -Sprint '5.0' -Name 'extensions-capture' -Status pass