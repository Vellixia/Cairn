#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint Phase 3.5: live container health + version sanity.
$resp = Test-Endpoint -Method GET -Path '/api/health'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/api/health returns 200'
$body = $resp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Eq -Expected 'cairn' -Actual $body.name -Msg '/api/health.name == cairn'
Assert-Eq -Expected 'ok'   -Actual $body.status -Msg '/api/health.status == ok'
Assert-Contains -Haystack $body.version -Needle '0.' -Msg '/api/health.version starts with 0.'

Show-Scenario -Sprint '3.5' -Name 'health' -Status pass
