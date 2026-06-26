#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 5.0+: Sessions HTTP API — create, patch, get, list, latest.

# --- POST /api/sessions — create ---
$createResp = Test-Endpoint -Method POST -Path '/api/sessions' -Body @{ project_hash = 'e2e-test-session' }
Assert-Eq -Expected 200 -Actual $createResp.StatusCode -Msg 'POST /api/sessions returns 200'
$session = $createResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
$sessionId = $session.id
Assert-True -Condition ($null -ne $sessionId) -Msg 'created session has an id'
Assert-Eq -Expected 'e2e-test-session' -Actual $session.project_hash -Msg 'session project_hash matches'

# --- PATCH /api/sessions/:id — update ---
$patchResp = Test-Endpoint -Method PATCH -Path "/api/sessions/$sessionId" -Body @{
    tasks = @('e2e test task')
    findings = @('found something')
    end = $false
}
Assert-Eq -Expected 200 -Actual $patchResp.StatusCode -Msg "PATCH /api/sessions/$sessionId returns 200"
$patched = $patchResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($patched.tasks.Count -ge 1) -Msg 'patched session has tasks'
Assert-Eq -Expected 'e2e test task' -Actual $patched.tasks[0].task -Msg 'patched task content matches'

# --- GET /api/sessions/:id ---
$getResp = Test-Endpoint -Method GET -Path "/api/sessions/$sessionId"
Assert-Eq -Expected 200 -Actual $getResp.StatusCode -Msg "GET /api/sessions/$sessionId returns 200"
$fetched = $getResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Eq -Expected $sessionId -Actual $fetched.id -Msg 'GET session returns matching id'

# --- PATCH /api/sessions/:id — close ---
$closeResp = Test-Endpoint -Method PATCH -Path "/api/sessions/$sessionId" -Body @{ end = $true }
Assert-Eq -Expected 200 -Actual $closeResp.StatusCode -Msg 'PATCH /api/sessions/:id close returns 200'
$closed = $closeResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($closed.ended_at -ne $null) -Msg 'closed session has ended_at set'

# --- GET /api/sessions/latest ---
$latestResp = Test-Endpoint -Method GET -Path '/api/sessions/latest'
Assert-Eq -Expected 200 -Actual $latestResp.StatusCode -Msg 'GET /api/sessions/latest returns 200'
$latestBody = $latestResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($null -ne $latestBody.session) -Msg 'latest endpoint returns a session'

# --- GET /api/sessions — list ---
$listResp = Test-Endpoint -Method GET -Path '/api/sessions'
Assert-Eq -Expected 200 -Actual $listResp.StatusCode -Msg 'GET /api/sessions returns 200'
$listBody = $listResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($listBody.Count -ge 1) -Msg 'session list is non-empty'
Assert-True -Condition ($listBody[0].id -eq $sessionId) -Msg 'session list contains the created session'

Show-Scenario -Sprint '5.0' -Name 'http-sessions' -Status pass
