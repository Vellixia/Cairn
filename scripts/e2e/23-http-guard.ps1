#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 5.0+: Guard HTTP API — verify, anchor, checkpoint, drift, approve.

# --- POST /api/guard/anchor — set ---
$anchorSet = Test-Endpoint -Method POST -Path '/api/guard/anchor' -Body @{ goal = 'e2e-http-guard: test guard flow' }
Assert-Eq -Expected 200 -Actual $anchorSet.StatusCode -Msg 'POST /api/guard/anchor returns 200'

# --- GET /api/guard/anchor — read back ---
$anchorGet = Test-Endpoint -Method GET -Path '/api/guard/anchor'
Assert-Eq -Expected 200 -Actual $anchorGet.StatusCode -Msg 'GET /api/guard/anchor returns 200'
$anchorBody = $anchorGet.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Contains -Haystack $anchorBody.anchor -Needle 'e2e-http-guard' -Msg 'anchor get returns the set goal'

# --- POST /api/guard/verify — clean content ---
$verifyResp = Test-Endpoint -Method POST -Path '/api/guard/verify' -Body @{
    path = 'Cargo.toml'
    content = 'workspace = { members = ["crates/*"] }'
}
Assert-Eq -Expected 200 -Actual $verifyResp.StatusCode -Msg 'POST /api/guard/verify returns 200'
$verifyBody = $verifyResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Eq -Expected 'ok' -Actual $verifyBody.risk -Msg 'verify clean reports risk = ok'

# --- POST /api/guard/verify — dangerous content (>50% removal) ---
$dangerResp = Test-Endpoint -Method POST -Path '/api/guard/verify' -Body @{
    path = 'Cargo.toml'
    content = '# nuked'
}
Assert-Eq -Expected 200 -Actual $dangerResp.StatusCode -Msg 'POST /api/guard/verify (danger) returns 200'
$dangerBody = $dangerResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($dangerBody.risk -in 'warn','danger') -Msg "verify dangerous reports warn or danger (got $($dangerBody.risk))"

# --- GET /api/guard/drift — verify drift was recorded ---
$driftResp = Test-Endpoint -Method GET -Path '/api/guard/drift'
Assert-Eq -Expected 200 -Actual $driftResp.StatusCode -Msg 'GET /api/guard/drift returns 200'
$driftBody = $driftResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($driftBody.Count -ge 1) -Msg 'drift list has at least 1 entry'

# Approve the first pending drift event.
$firstDriftId = $driftBody[0].id
Assert-True -Condition ($null -ne $firstDriftId) -Msg 'drift entry has an id'

$approveResp = Test-Endpoint -Method POST -Path "/api/guard/drift/$firstDriftId/approve"
Assert-Eq -Expected 200 -Actual $approveResp.StatusCode -Msg "POST /api/guard/drift/$firstDriftId/approve returns 200"
$approveBody = $approveResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Eq -Expected 'approved' -Actual $approveBody.status -Msg 'drift approve confirms status'

# --- POST /api/guard/checkpoint ---
$cpResp = Test-Endpoint -Method POST -Path '/api/guard/checkpoint'
Assert-Eq -Expected 200 -Actual $cpResp.StatusCode -Msg 'POST /api/guard/checkpoint returns 200'

# --- GET /api/guard/checkpoints ---
$cpsResp = Test-Endpoint -Method GET -Path '/api/guard/checkpoints'
Assert-Eq -Expected 200 -Actual $cpsResp.StatusCode -Msg 'GET /api/guard/checkpoints returns 200'
$cpsBody = $cpsResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($cpsBody.Count -ge 1) -Msg 'checkpoints list is non-empty'

Show-Scenario -Sprint '5.0' -Name 'http-guard' -Status pass
