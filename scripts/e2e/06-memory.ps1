#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 2-3: memory CRUD — remember, recall, wakeup, edit, pin, reinforce.
$out = Invoke-CairnCli remember 'e2e-test: cairn uses rust and helixdb'
Assert-Contains -Haystack $out -Needle 'remembered' -Msg 'remember returns remembered'

$out2 = Invoke-CairnCli recall 'rust helixdb' --limit 5
Assert-Contains -Haystack $out2 -Needle 'rust' -Msg 'recall finds the seeded memory'

$out3 = Invoke-CairnCli wakeup --limit 3
Assert-True -Condition ($out3.Length -gt 0) -Msg 'wakeup returns content'

# memory_edit via HTTP
$resp = Test-Endpoint -Method POST -Path '/api/tools/call' -Body @{
    name = 'memory_edit'
    arguments = @{ id = 'nonexistent'; content = 'test' }
}
Assert-Eq -Expected 200 -Actual $resp.StatusCode -Msg 'memory_edit returns 200'
$text = $resp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-Contains -Haystack $text.content[0].text -Needle 'error' -Msg 'memory_edit on nonexistent id returns error'

Show-Scenario -Sprint '3.5' -Name 'memory' -Status pass
