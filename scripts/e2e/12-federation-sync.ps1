#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 14: federation — trust scopes + revocations endpoint.
$resp = Test-Endpoint -Method GET -Path '/registry/trusted-keys'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/registry/trusted-keys returns 200'

# /registry/revocations returns 200 when the log exists, 400 when it
# doesn't (fresh install). Both are valid smoke-test results.
$resp2 = Test-Endpoint -Method GET -Path '/registry/revocations?since=0'
Assert-True -Condition ($resp2.StatusCode -in 200,400) -Msg "/registry/revocations returns 200 or 400 (got $($resp2.StatusCode))"

Show-Scenario -Sprint '4.1' -Name 'federation-sync' -Status pass
