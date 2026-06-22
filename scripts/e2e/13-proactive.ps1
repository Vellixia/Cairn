#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 18: proactive recall — fires on cue, skips plain, respects opt-out.
# Seed a memory.
Invoke-CairnCli remember 'e2e-proactive: last time we picked tabs over spaces'

# Recall cue prompt — the hook should fire.
$resp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'proactive_recall'
    arguments = @{ prompt = 'What did we decide last time about formatting?' }
}
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg 'proactive_recall returns 200'
$text = $resp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
$content = $text.content[0].text
Assert-Contains -Haystack $content -Needle 'tabs' -Msg 'proactive_recall returns seeded memory on recall cue'

# Plain imperative — should return empty.
$resp2 = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'proactive_recall'
    arguments = @{ prompt = 'Add a print statement' }
}
Assert-Eq -Expected 200 -Actual $resp2.StatusCode -Msg 'proactive_recall plain imperative returns 200'
$text2 = $resp2.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
$content2 = $text2.content[0].text
Assert-Eq -Expected '[]' -Actual $content2.Trim() -Msg 'proactive_recall returns [] on plain imperative'

Show-Scenario -Sprint '5.0' -Name 'proactive' -Status pass
