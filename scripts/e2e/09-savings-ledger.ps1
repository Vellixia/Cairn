#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 5: savings ledger — /api/ledger + /api/ledger/verify.
$resp = Test-Endpoint -Method GET -Path '/api/ledger'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/api/ledger returns 200'

# /api/ledger/verify returns 200 when the ledger chain is valid, 400 when
# there are no entries yet. Both are acceptable smoke-test results.
$resp2 = Test-Endpoint -Method GET -Path '/api/ledger/verify'
Assert-True -Condition ($resp2.StatusCode -in 200,400) -Msg "/api/ledger/verify returns 200 or 400 (got $($resp2.StatusCode))"

Show-Scenario -Sprint '3.5' -Name 'savings-ledger' -Status pass
