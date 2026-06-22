#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 20: PWA + push — sw.js reachable, /api/push/subscribe round-trip.
$resp = Test-Endpoint -Method GET -Path '/sw.js'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/sw.js returns 200'
# The service worker file is served from the embedded web/out. It should
# contain a `self.addEventListener` or `install` keyword — any of which
# proves it's a service worker and not a generic JS file.
$hasSw = $resp.Body.Contains('addEventListener') -or $resp.Body.Contains('serviceWorker') -or $resp.Body.Contains('install')
Assert-True -Condition $hasSw -Msg '/sw.js looks like a service worker'

$resp2 = Test-Endpoint -Method POST -Path '/api/push/subscribe' -Body @{
    endpoint = 'https://push.example/sub/e2e'
    keys = @{ p256dh = 'pp'; auth = 'aa' }
    user_agent = 'e2e-test'
}
Assert-Eq -Expected 201 -Actual $resp2.StatusCode -Msg '/api/push/subscribe returns 201'

$resp3 = Test-Endpoint -Method GET -Path '/api/push/list'
Assert-Eq -Expected 200 -Actual $resp3.StatusCode -Msg '/api/push/list returns 200'

Show-Scenario -Sprint '5.0' -Name 'pwa-push' -Status pass
