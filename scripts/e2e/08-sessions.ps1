#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 4: sessions — list via HTTP.
# /api/sessions returns a JSON array (possibly empty). The local store
# in the container may have no sessions yet, so the response is `[]`.
$resp = Test-Endpoint -Method GET -Path '/api/sessions'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/api/sessions returns 200'
# ConvertFrom-Json on `[]` returns $null; check the raw body instead.
Assert-Contains -Haystack $resp.Body.Trim() -Needle '[' -Msg '/api/sessions returns a JSON array'

# Sprint 4: drift — list pending.
$resp2 = Test-Endpoint -Method GET -Path '/api/drift?status=pending'
Assert-Eq -Expected 200 -Actual $resp2.StatusCode -Msg '/api/drift returns 200'

Show-Scenario -Sprint '3.5' -Name 'sessions-drift' -Status pass
