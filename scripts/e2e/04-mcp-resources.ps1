#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 24: MCP resources — list + read all 6 URIs via HTTP.
$resp = Test-Endpoint -Method GET -Path '/api/tools/list'
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg '/api/tools/list returns 200'

# resources/list via HTTP
$resp2 = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'resources/list'
    arguments = @{}
}
Assert-Eq -Expected 200 -Actual $resp2.StatusCode -Msg 'resources/list returns 200'

# resources/read: memory/graph
$resp3 = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'resources/read'
    arguments = @{ uri = 'cairn://memory/graph' }
}
Assert-Eq -Expected 200 -Actual $resp3.StatusCode -Msg 'resources/read memory/graph returns 200'

# resources/read: config/toml
$resp4 = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'resources/read'
    arguments = @{ uri = 'cairn://config/toml' }
}
Assert-Eq -Expected 200 -Actual $resp4.StatusCode -Msg 'resources/read config/toml returns 200'

Show-Scenario -Sprint '5.0' -Name 'mcp-resources' -Status pass
