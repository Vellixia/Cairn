#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 7: hybrid search — /api/search endpoint.
$resp = Test-Endpoint -Method GET -Path '/api/search?q=cairn&limit=3'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/api/search returns 200'
$body = $resp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($null -ne $body) -Msg '/api/search returns JSON'

Show-Scenario -Sprint '3.5' -Name 'hybrid-search' -Status pass
