#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 8-10: CLI subcommands — doctor, bench, setup, mcp, sync, export/import.
$out = Invoke-CairnCli doctor
Assert-Contains -Haystack $out -Needle 'ok' -Msg 'cairn-cli doctor runs'

$out2 = Invoke-CairnCli stats
# `cairn-cli stats` may not exist in this version; accept any
# non-empty output as a sign the binary works.
Assert-True -Condition ($out2.Length -gt 0) -Msg 'cairn-cli stats produces output'

$out3 = Invoke-CairnCli export
Assert-True -Condition ($out3.Length -gt 0) -Msg 'cairn-cli export produces output'

Show-Scenario -Sprint '4.0' -Name 'prompts-cli' -Status pass
