#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 11: pack registry — /registry/packs endpoint.
# The registry returns 200 with [] (empty array) when no packs are
# published, or 200 with the pack list. Both are valid smoke-test
# results.
$resp = Test-Endpoint -Method GET -Path '/registry/packs'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/registry/packs returns 200'
# Just verify the body starts with `[` (a JSON array) — ConvertFrom-Json
# on `[]` returns $null which would fail a `$null -ne $body` check.
Assert-Contains -Haystack $resp.Body.Trim() -Needle '[' -Msg '/registry/packs returns a JSON array'

# Sprint 13: registry search.
$resp2 = Test-Endpoint -Method GET -Path '/registry/search?q=test'
Assert-Eq -Expected 200 -Actual $resp2.StatusCode -Msg '/registry/search returns 200'

Show-Scenario -Sprint '4.1' -Name 'registry-pack' -Status pass
