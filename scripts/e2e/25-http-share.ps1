#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 5.0+: Share HTTP API — sanitize, export, import round-trip.

# --- POST /api/share/sanitize ---
$sanitizeResp = Test-Endpoint -Method POST -Path '/api/share/sanitize' -Body @{
    text = 'deploy token=ghp_abcdef123456789012345678901234567890 and password=supersecret'
}
Assert-Eq -Expected 200 -Actual $sanitizeResp.StatusCode -Msg 'POST /api/share/sanitize returns 200'
$sanitized = $sanitizeResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Eq -Expected 'needs_review' -Actual $sanitized.classification -Msg 'sanitize classifies as needs_review'
Assert-True -Condition ($sanitized.text -match 'REDACTED|redact') -Msg 'sanitize redacts the token in output'

# --- GET /api/share/export ---
$exportResp = Test-Endpoint -Method GET -Path '/api/share/export'
Assert-Eq -Expected 200 -Actual $exportResp.StatusCode -Msg 'GET /api/share/export returns 200'
$exportBody = $exportResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($null -ne $exportBody.memories) -Msg 'export returns memories array'
Assert-True -Condition ($exportBody.schema -match 'cairn') -Msg 'export includes schema info'

# Export the sanitized text for import test.
$exportJson = $exportResp.Body

# --- POST /api/share/import — re-import the export ---
$importResp = Test-Endpoint -Method POST -Path '/api/share/import' -Body @{
    schema = if ($exportBody.schema) { $exportBody.schema } else { 'cairn/share/v1' }
    version = if ($exportBody.version) { $exportBody.version } else { 1 }
    total = $exportBody.memories.Count
    shared = $exportBody.shared
    needs_review = $exportBody.needs_review
    withheld = $exportBody.withheld
    memories = $exportBody.memories
}
Assert-Eq -Expected 200 -Actual $importResp.StatusCode -Msg 'POST /api/share/import returns 200'
$importBody = $importResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($importBody.ingested -ge 0) -Msg 'import reports ingested count (may be 0 if no shareable memories)'

Show-Scenario -Sprint '5.0' -Name 'http-share' -Status pass
