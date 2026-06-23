#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 19a: multi-tenant — org_id isolation smoke.
# The container runs with multi_tenant=false by default, so we verify
# the default org path works and the endpoint exists.
$resp = Test-Endpoint -Method GET -Path '/api/metrics'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/api/metrics returns 200 with cookie'
$body = $resp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
# /api/metrics returns a JSON object; just verify it parses.
Assert-True -Condition ($null -ne $body) -Msg '/api/metrics returns parseable JSON'

Show-Scenario -Sprint '5.0' -Name 'multi-tenant' -Status pass
