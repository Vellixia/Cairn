#Requires -Version 5.1
. "$PSScriptRoot/_lib.ps1"

# Sprint 5.0+: Edge cases — concurrent writes, large payload, CORS, non-loopback rejection.

# --- Concurrent writes (two rapid remember calls) ---
$job1 = Start-Job -ScriptBlock { param($d,$m) & "$d/target/release/cairn.exe" remember $m --data-dir "$d/.e2e-data" } -ArgumentList $script:RepoRoot, 'e2e-edge-concurrent-A'
$job2 = Start-Job -ScriptBlock { param($d,$m) & "$d/target/release/cairn.exe" remember $m --data-dir "$d/.e2e-data" } -ArgumentList $script:RepoRoot, 'e2e-edge-concurrent-B'
$j1out = $job1 | Receive-Job -Wait -AutoRemoveJob
$j2out = $job2 | Receive-Job -Wait -AutoRemoveJob
Assert-Contains -Haystack $j1out -Needle 'remembered' -Msg 'concurrent remember A succeeds'
Assert-Contains -Haystack $j2out -Needle 'remembered' -Msg 'concurrent remember B succeeds'

# Verify both are recallable.
$recallOut = Invoke-CairnCli recall 'e2e-edge-concurrent' --limit 5
Assert-Contains -Haystack $recallOut -Needle 'concurrent-A' -Msg 'recall finds concurrent-A'
Assert-Contains -Haystack $recallOut -Needle 'concurrent-B' -Msg 'recall finds concurrent-B'

# --- Large payload (10KB memory) ---
$bigText = "e2e-edge-large-payload " + ("x" * 10000)
$bigResp = Invoke-CairnCli remember $bigText
Assert-Contains -Haystack $bigResp -Needle 'remembered' -Msg '10KB memory is stored'

$bigRecall = Invoke-CairnCli recall 'e2e-edge-large-payload' --limit 3
Assert-Contains -Haystack $bigRecall -Needle 'e2e-edge-large-payload' -Msg 'large payload is recallable'

# --- Extensions capture with non-loopback Origin must be rejected ---
$captureBody = @{
    kind = 'selection'
    url = 'https://evil.com/phish'
    title = 'Phish'
    text = 'stolen data'
    captured_at = '2026-06-26T00:00:00Z'
} | ConvertTo-Json -Compress
$tmp = New-TemporaryFile
try {
    $code = & curl.exe -sS -o $tmp.FullName -w '%{http_code}' `
        -X POST -H 'content-type: application/json' `
        -H "Cookie: $($Global:E2E_Cookie)" `
        -H 'Origin: https://evil.com' `
        -d $captureBody "$Global:E2E_BaseUrl/api/extensions/capture"
    Assert-Eq -Expected 403 -Actual ([int]$code) -Msg 'non-loopback extension capture returns 403'
} finally {
    Remove-Item $tmp -ErrorAction SilentlyContinue
}

# --- CORS preflight (OPTIONS) ---
$tmp2 = New-TemporaryFile
try {
    $code = & curl.exe -sS -o $tmp2.FullName -w '%{http_code}' `
        -X OPTIONS -H 'Origin: https://example.com' `
        -H 'Access-Control-Request-Method: POST' `
        "$Global:E2E_BaseUrl/api/health"
    $headers = Get-Content $tmp2.FullName -Raw
    Assert-Eq -Expected 200 -Actual ([int]$code) -Msg 'OPTIONS /api/health returns 200'
    Assert-Contains -Haystack $headers -Needle 'Access-Control-Allow-Origin' -Msg 'CORS preflight returns allow-origin header'
} finally {
    Remove-Item $tmp2 -ErrorAction SilentlyContinue
}

# --- CSP headers present on HTML responses ---
$tmp3 = New-TemporaryFile
try {
    & curl.exe -sS -o $tmp3.FullName -D - "$Global:E2E_BaseUrl/" | Out-Null
    $headers = Get-Content $tmp3.FullName -Raw
    Assert-Contains -Haystack $headers -Needle 'content-security-policy' -Msg 'root page includes CSP header'
} finally {
    Remove-Item $tmp3 -ErrorAction SilentlyContinue
}

Show-Scenario -Sprint '5.0' -Name 'edge-cases' -Status pass
