#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 22: transcript ingestion — /api/ingest/transcript with VTT.
$vtt = @"
WEBVTT

00:00:01.000 --> 00:00:03.500
Hello world

00:00:04.000 --> 00:00:06.000
<v Alice>Hi back</v>
"@
$resp = Test-Endpoint -Method POST -Path '/api/ingest/transcript' -Body @{
    body = $vtt
    format = 'vtt'
    source_url = 'https://example.com/meeting'
}
Assert-Eq -Expected 201 -Actual $resp.StatusCode -Msg '/api/ingest/transcript VTT returns 201'
$body = $resp.Body | ConvertFrom-Json -ErrorAction SilentlyContinue
Assert-True -Condition ($body.chunks_written -ge 1) -Msg 'VTT ingest produces at least 1 chunk'
Assert-True -Condition ($body.memory_ids.Count -ge 1) -Msg 'VTT ingest returns memory_ids'

# SRT format.
$srt = "1`n00:00:01,000 --> 00:00:03,500`nfirst cue`n`n2`n00:00:04,000 --> 00:00:06,000`nsecond cue`n"
$resp2 = Test-Endpoint -Method POST -Path '/api/ingest/transcript' -Body @{
    body = $srt
    format = 'srt'
}
Assert-Eq -Expected 201 -Actual $resp2.StatusCode -Msg '/api/ingest/transcript SRT returns 201'

Show-Scenario -Sprint '5.0' -Name 'ingest-transcript' -Status pass
