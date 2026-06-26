#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 5.0+: MCP tool depth — test every tool that isn't already covered
# by scenarios 03-06 (proactive_recall, search, graph status, resources, prompts,
# basic remember/recall/wakeup).

# --- read + expand round-trip ---
$readResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'read'
    arguments = @{ path = 'Cargo.toml'; mode = 'map' }
}
Assert-Eq -Expected 200 -Actual $readResp.StatusCode -Msg 'read Cargo.toml returns 200'
$readBody = $readResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($readBody.content[0].text -match 'cairn-core') -Msg 'read output contains cairn-core'

# Capture the content hash for expand test.
$readText = $readBody.content[0].text

# --- verify (clean content) ---
$verifyResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'verify'
    arguments = @{ path = 'Cargo.toml'; content = $readText }
}
Assert-Eq -Expected 200 -Actual $verifyResp.StatusCode -Msg 'verify clean Cargo.toml returns 200'
$verifyBody = $verifyResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Contains -Haystack $verifyBody.content[0].text -Needle 'ok' -Msg 'verify clean reports ok'

# --- anchor set + get round-trip ---
$anchorSet = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'anchor'
    arguments = @{ goal = 'e2e-test: verify MCP tool depth' }
}
Assert-Eq -Expected 200 -Actual $anchorSet.StatusCode -Msg 'anchor set returns 200'

$anchorGet = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'anchor'
    arguments = @{}
}
Assert-Eq -Expected 200 -Actual $anchorGet.StatusCode -Msg 'anchor get returns 200'
$anchorBody = $anchorGet.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Contains -Haystack $anchorBody.content[0].text -Needle 'e2e-test' -Msg 'anchor get returns the set goal'

# --- prefer + profile round-trip ---
$preferResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'prefer'
    arguments = @{ rule = 'e2e-test: use tabs for indentation' }
}
Assert-Eq -Expected 200 -Actual $preferResp.StatusCode -Msg 'prefer returns 200'

$profileResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'profile'
    arguments = @{}
}
Assert-Eq -Expected 200 -Actual $profileResp.StatusCode -Msg 'profile returns 200'
$profileBody = $profileResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Contains -Haystack $profileBody.content[0].text -Needle 'e2e-test' -Msg 'profile shows the set preference'

# --- assemble ---
$assembleResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'assemble'
    arguments = @{ query = 'cairn'; budget = 500 }
}
Assert-Eq -Expected 200 -Actual $assembleResp.StatusCode -Msg 'assemble returns 200'
$assembleBody = $assembleResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($assembleBody.content[0].text.Length -gt 0) -Msg 'assemble returns non-empty content'

# --- sanitize ---
$sanitizeResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'sanitize'
    arguments = @{ text = 'my api key is sk-abc123def456 and should be redacted' }
}
Assert-Eq -Expected 200 -Actual $sanitizeResp.StatusCode -Msg 'sanitize returns 200'
$sanitizeBody = $sanitizeResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Contains -Haystack $sanitizeBody.content[0].text -Needle 'REDACTED' -Msg 'sanitize redacts sk-... token'

# --- compress ---
$compressResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'compress'
    arguments = @{
        command = 'cargo build'
        output = "Compiling foo v1.0`nCompiling bar v2.0`n    Finished dev [unoptimized] in 0.5s"
    }
}
Assert-Eq -Expected 200 -Actual $compressResp.StatusCode -Msg 'compress returns 200'
$compressBody = $compressResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Contains -Haystack $compressBody.content[0].text -Needle 'compiled 2 crates' -Msg 'compress consolidates cargo build output'

# Seed a memory for memory tool tests.
$seedResp = Invoke-CairnCli remember 'e2e-mcp-depth: the answer is 42'
Assert-Contains -Haystack $seedResp -Needle 'remembered' -Msg 'seed memory for mcp-depth tools'

# Retrieve the memory id from recall.
$recallOut = Invoke-CairnCli recall 'e2e-mcp-depth answer 42' --limit 3
$memId = $null
if ($recallOut -match '(\w{8}-\w{4}-\w{4}-\w{4}-\w{12})') { $memId = $Matches[1] }
Assert-True -Condition ($null -ne $memId) -Msg 'recall found the seeded memory with a UUID'

# --- memory_pin ---
$pinResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'memory_pin'
    arguments = @{ id = $memId; pinned = $true }
}
Assert-Eq -Expected 200 -Actual $pinResp.StatusCode -Msg 'memory_pin returns 200'
$pinBody = $pinResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Contains -Haystack $pinBody.content[0].text -Needle 'pinned' -Msg 'memory_pin confirms pinned'

# --- memory_reinforce ---
$reinforceResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'memory_reinforce'
    arguments = @{ id = $memId }
}
Assert-Eq -Expected 200 -Actual $reinforceResp.StatusCode -Msg 'memory_reinforce returns 200'

# --- memory_timeline ---
$timelineResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'memory_timeline'
    arguments = @{ limit = 5 }
}
Assert-Eq -Expected 200 -Actual $timelineResp.StatusCode -Msg 'memory_timeline returns 200'
$timelineBody = $timelineResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($timelineBody.content[0].text.Length -gt 0) -Msg 'memory_timeline returns content'

# --- memory_crystallize ---
$crystalResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'memory_crystallize'
    arguments = @{}
}
Assert-Eq -Expected 200 -Actual $crystalResp.StatusCode -Msg 'memory_crystallize returns 200'

# --- memory_promote ---
$promoteResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'memory_promote'
    arguments = @{ id = $memId; tier = 'semantic' }
}
Assert-Eq -Expected 200 -Actual $promoteResp.StatusCode -Msg 'memory_promote returns 200'
$promoteBody = $promoteResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Contains -Haystack $promoteBody.content[0].text -Needle 'semantic' -Msg 'memory_promote confirms semantic tier'

# --- checkpoint + checkpoints (no rollback — don't disturb workspace) ---
$cpResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'checkpoint'
    arguments = @{ label = 'e2e-mcp-depth-checkpoint' }
}
Assert-Eq -Expected 200 -Actual $cpResp.StatusCode -Msg 'checkpoint returns 200'

$cpsResp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'checkpoints'
    arguments = @{}
}
Assert-Eq -Expected 200 -Actual $cpsResp.StatusCode -Msg 'checkpoints returns 200'
$cpsBody = $cpsResp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Contains -Haystack $cpsBody.content[0].text -Needle 'e2e-mcp-depth' -Msg 'checkpoints lists the created checkpoint'

Show-Scenario -Sprint '5.0' -Name 'mcp-depth' -Status pass
