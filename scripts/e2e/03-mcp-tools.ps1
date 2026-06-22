#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 10: MCP tools — list + key tools respond via HTTP /api/tools/call.
$resp = Test-Endpoint -Method GET -Path '/api/tools/list'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/api/tools/list returns 200'
$body = $resp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
$tools = $body.tools
Assert-True -Condition ($tools.Count -ge 29) -Msg "tools/list count >= 29 (got $($tools.Count))"

# tools/call: proactive_recall with a recall cue
$resp2 = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'proactive_recall'
    arguments = @{ prompt = 'What did we decide last time?' }
}
Assert-Eq -Expected 200 -Actual $resp2.StatusCode -Msg '/api/tools/call proactive_recall returns 200'
$text = $resp2.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($text.content[0].text -ne $null) -Msg 'proactive_recall returns content'

# tools/call: search
$resp3 = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'search'
    arguments = @{ query = 'cairn'; limit = 3 }
}
Assert-Eq -Expected 200 -Actual $resp3.StatusCode -Msg '/api/tools/call search returns 200'

# tools/call: graph status
$resp4 = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'graph'
    arguments = @{ action = 'status' }
}
Assert-Eq -Expected 200 -Actual $resp4.StatusCode -Msg '/api/tools/call graph status returns 200'

Show-Scenario -Sprint '4.0' -Name 'mcp-tools' -Status pass
