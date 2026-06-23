#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 24: MCP prompts — list + get all 5 via HTTP.
$resp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'prompts/list'
    arguments = @{}
}
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg 'prompts/list returns 200'

# prompts/get: summarize-drift
$resp2 = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'prompts/get'
    arguments = @{ name = 'summarize-drift' }
}
Assert-Eq -Expected 200 -Actual $resp2.StatusCode -Msg 'prompts/get summarize-drift returns 200'

Show-Scenario -Sprint '5.0' -Name 'mcp-prompts' -Status pass
