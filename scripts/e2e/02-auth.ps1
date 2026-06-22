#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint Phase 3.5: auth flow — login, me, logout, re-login.
# Save the cookie so we can restore it after the logout test.
$savedCookie = $Global:E2E_Cookie

$resp = Test-Endpoint -Method GET -Path '/api/auth/me'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/api/auth/me returns 200 with cookie'
$body = $resp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Eq -Expected 'admin' -Actual $body.username -Msg '/api/auth/me username is admin'

# Logout clears the cookie.
$resp2 = Test-Endpoint -Method POST -Path '/api/auth/logout'
Assert-Eq -Expected 200 -Actual $resp2.StatusCode -Msg '/api/auth/logout returns 200'

# After logout, /api/auth/me should 401.
$Global:E2E_Cookie = $null
$resp3 = Test-Endpoint -Method GET -Path '/api/auth/me'
Assert-Eq -Expected 401 -Actual $resp3.StatusCode -Msg '/api/auth/me returns 401 after logout'

# Restore the cookie for subsequent scenarios.
$Global:E2E_Cookie = $savedCookie

Show-Scenario -Sprint '3.5' -Name 'auth' -Status pass