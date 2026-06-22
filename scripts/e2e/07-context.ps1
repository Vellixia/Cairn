#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 3.5: context tools — read, verify, checkpoint, rollback, sanitize.
$out = Invoke-CairnCli read Cargo.toml --mode map
Assert-Contains -Haystack $out -Needle 'cairn' -Msg 'read Cargo.toml returns content'

# sanitize: the test token format matters. `ghp_` is the GitHub PAT
# prefix. The sanitizer may have updated patterns; accept any token
# redaction marker.
$out2 = Invoke-CairnCli sanitize 'my token is ghp_0123456789abcdefghijklmnopqrstuvwxyz'
$redacted = $out2 -match 'redact|REDACTED|\[github|\[redacted'
Assert-True -Condition $redacted -Msg 'sanitize redacts github token'

$out3 = Invoke-CairnCli checkpoints
Assert-True -Condition ($out3.Length -gt 0) -Msg 'checkpoints returns output'

Show-Scenario -Sprint '3.5' -Name 'context' -Status pass
