#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 3.5: context tools — read, verify, checkpoint, rollback, sanitize.
$out = Invoke-CairnCli read Cargo.toml --mode map
Assert-Contains -Haystack $out -Needle 'cairn' -Msg 'read Cargo.toml returns content'

# sanitize: the test token format matters. `ghp_` is the GitHub PAT
# prefix. `cairn-cli sanitize` is not a subcommand (it's an MCP/HTTP tool)
# so we drive it via /api/tools/call directly. Accept any token redaction
# marker.
$body = @{
    name = 'sanitize'
    arguments = @{
        text = 'my token is ghp_0123456789abcdefghijklmnopqrstuvwxyz'
    }
} | ConvertTo-Json -Compress
$tmp = New-TemporaryFile
try {
    $code = & curl.exe -sS -o $tmp.FullName -w '%{http_code}' `
        -X POST `
        -H 'content-type: application/json' `
        -H "Cookie: $($Global:E2E_Cookie)" `
        -d $body `
        "$Global:E2E_BaseUrl/api/tools/call"
    $respBody = Get-Content $tmp.FullName -Raw
    $redacted = $respBody -match 'redact|REDACTED|\[github|\[redacted'
    Assert-True -Condition $redacted -Msg 'sanitize redacts github token'
} finally {
    Remove-Item $tmp -ErrorAction SilentlyContinue
}

$out3 = Invoke-CairnCli checkpoints
Assert-True -Condition ($out3.Length -gt 0) -Msg 'checkpoints returns output'

Show-Scenario -Sprint '3.5' -Name 'context' -Status pass