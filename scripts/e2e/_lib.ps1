# scripts/e2e/_lib.ps1 — shared helpers for the Cairn end-to-end harness.
#
# Every scenario script dot-sources this file and uses its primitives:
#
#   Assert-Eq   -Expected X -Actual Y -Msg "..."
#   Assert-True -Condition X -Msg "..."
#   Test-Endpoint -Method GET -Path /api/health [-Token $token] [-Body $obj] -ExpectStatus 200
#   Invoke-Mcp  -Tool <name> -Args @{...}                  # drives cairn-cli.exe mcp over stdio
#   Initialize-Mcp                                              # one-time per session
#   Show-Scenario -Sprint <n> -Name <name> [-Status pass|fail|skip]
#   Get-BearerToken -Username admin -Password $pw

#Requires -Version 5.1
$ErrorActionPreference = 'Stop'

# ----------------------------------------------------------------------
# Paths
# ----------------------------------------------------------------------

$script:RepoRoot      = (Resolve-Path "$PSScriptRoot/../..").Path
$Global:E2E_BinCairn   = Join-Path $script:RepoRoot 'target/release/cairn.exe'
$Global:E2E_BinCairnCli = Join-Path $script:RepoRoot 'target/release/cairn-cli.exe'
$Global:E2E_BaseUrl   = if ($env:CAIRN_BASE_URL) { $env:CAIRN_BASE_URL } else { 'http://127.0.0.1:7777' }
$Global:E2E_DataDir   = if ($env:CAIRN_E2E_DATA) { $env:CAIRN_E2E_DATA } else { Join-Path $script:RepoRoot '.e2e-data' }
$Global:E2E_LogFile   = Join-Path $Global:E2E_DataDir 'e2e.log'

# Prepend our freshly-built release dir to PATH so `cairn-cli` resolves
# to v0.5.0 regardless of any older global copy.
$env:PATH = "$script:RepoRoot/target/release;$env:PATH"

# Ensure the data dir exists.
New-Item -ItemType Directory -Force -Path $Global:E2E_DataDir | Out-Null

# ----------------------------------------------------------------------
# Per-run counters
# ----------------------------------------------------------------------

if (-not (Get-Variable -Name 'E2E_Total' -Scope Global -ErrorAction SilentlyContinue)) {
    $Global:E2E_Total = 0
    $Global:E2E_Passed = 0
    $Global:E2E_Failed = 0
    $Global:E2E_Skipped = 0
    $Global:E2E_Rows = New-Object System.Collections.Generic.List[string]
}
$Global:E2E_FailFast = if ($env:CAIRN_E2E_FAILFAST -ne '0') { $true } else { $false }
$Global:E2E_Token = $Global:E2E_Token
$Global:E2E_BinCairn   = $Global:E2E_BinCairn
$Global:E2E_BinCairnCli = $Global:E2E_BinCairnCli
$Global:E2E_BaseUrl   = $Global:E2E_BaseUrl
$Global:E2E_DataDir   = $Global:E2E_DataDir
$Global:E2E_LogFile   = $Global:E2E_LogFile

function Write-Log {
    param([string]$Msg)
    Add-Content -Path $Global:E2E_LogFile -Value "[$(Get-Date -Format o)] $Msg"
}

# ----------------------------------------------------------------------
# Assertions
# ----------------------------------------------------------------------

function Assert-True {
    param(
        [Parameter(Mandatory = $true)] [bool] $Condition,
        [Parameter(Mandatory = $true)] [string] $Msg
    )
    $Global:E2E_Total++
    if ($Condition) {
        $Global:E2E_Passed++
        Write-Log "PASS: $Msg"
        return
    }
    $Global:E2E_Failed++
    Write-Log "FAIL: $Msg"
    if ($Global:E2E_FailFast) {
        throw "FAIL (fail-fast): $Msg"
    }
}

function Assert-Eq {
    param(
        [Parameter(Mandatory = $true)] $Expected,
        [Parameter(Mandatory = $true)] $Actual,
        [Parameter(Mandatory = $true)] [string] $Msg
    )
    $eq = ($Expected -eq $Actual)
    if (-not $eq) {
        Write-Log "FAIL diff: expected=[$Expected] actual=[$Actual] msg=[$Msg]"
    }
    Assert-True -Condition $eq -Msg "$Msg (expected=[$Expected], got=[$Actual])"
}

function Assert-Contains {
    param(
        [Parameter(Mandatory = $true)] $Haystack,
        [Parameter(Mandatory = $true)] $Needle,
        [Parameter(Mandatory = $true)] [string] $Msg
    )
    $hit = ($Haystack | Out-String).Contains([string]$Needle)
    Assert-True -Condition $hit -Msg "$Msg (looking for [$Needle])"
}

# ----------------------------------------------------------------------
# HTTP
# ----------------------------------------------------------------------

function Test-Endpoint {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)] [ValidateSet('GET','POST','PUT','DELETE','PATCH')] [string] $Method,
        [Parameter(Mandatory = $true)] [string] $Path,
        [string] $Token,
        $Body,
        [int] $ExpectStatus = 200,
        [int] $TimeoutSec = 30
    )
    $url = "$Global:E2E_BaseUrl$Path"
    $args = @('-sS', '-o', 'response.txt', '-w', '%{http_code}', '--max-time', "$TimeoutSec", '-X', $Method, $url)
    # Send whichever auth is available: explicit Token, then the captured
    # login cookie, then nothing.
    $auth = $null
    if ($Token) {
        $auth = "Authorization: Bearer $Token"
    } elseif ($Global:E2E_Cookie) {
        $auth = "Cookie: $($Global:E2E_Cookie)"
        Write-Log "Test-Endpoint: sending cookie len=$($Global:E2E_Cookie.Length) path=$Path"
    } else {
        Write-Log "Test-Endpoint: NO AUTH path=$Path"
    }
    if ($auth) {
        $args += @('-H', $auth)
    }
    if ($PSBoundParameters.ContainsKey('Body') -and $null -ne $Body) {
        $json = $Body | ConvertTo-Json -Depth 16 -Compress
        $args += @('-H', 'content-type: application/json', '-d', $json)
    }
    $code = & curl.exe @args
    $response = if (Test-Path response.txt) { Get-Content response.txt -Raw } else { '' }
    Remove-Item response.txt -ErrorAction SilentlyContinue
    return [pscustomobject]@{
        StatusCode = [int]$code
        Body       = $response
    }
}

function Get-BearerToken {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)] [string] $Username,
        [Parameter(Mandatory = $true)] [string] $Password
    )
    # The cairn-server /api/auth/login endpoint sets the session via
    # Set-Cookie; there's no JSON bearer field. We capture the cookie
    # from the response headers and stash it in $Global:E2E_Cookie so
    # subsequent Test-Endpoint calls can authenticate.
    $body = @{ username = $Username; password = $Password } | ConvertTo-Json -Compress
    $bodyPath = New-TemporaryFile
    $hdrPath  = New-TemporaryFile
    try {
        Set-Content -Path $bodyPath.FullName -Value $body -Encoding ASCII -NoNewline
        $code = & curl.exe -sS -o $bodyPath.FullName -D $hdrPath.FullName -w '%{http_code}' `
            -H 'content-type: application/json' `
            --data-binary "@$($bodyPath.FullName)" `
            "$Global:E2E_BaseUrl/api/auth/login"
        if ($code -ne '200') {
            $err = Get-Content $bodyPath.FullName -Raw
            throw "login failed: HTTP $code body=$err"
        }
        # Extract the cairn_session cookie. curl.exe -D on Windows outputs
        # headers as a single space-separated line, not one per line.
        $raw = Get-Content $hdrPath.FullName -Raw
        if ($raw -notmatch 'set-cookie:\s*([^;]+)') {
            throw "login response has no Set-Cookie header (raw=$raw)"
        }
        $cookie = $Matches[1].Trim()
        $Global:E2E_Cookie = $cookie
        Write-Log "login OK; cookie len=$($cookie.Length)"
        return $cookie
    } finally {
        Remove-Item $bodyPath, $hdrPath -ErrorAction SilentlyContinue
    }
}

# ----------------------------------------------------------------------
# MCP stdio driver
#
# Spawns cairn-cli mcp with redirected stdin/stdout, sends a JSON-RPC
# message per call, parses the response. The cairn-cli stdio server reads
# one JSON message per line, replies with one JSON message per line.
# ----------------------------------------------------------------------

$Global:E2E_McpProc = $null
$Global:E2E_McpId = 0

function Initialize-Mcp {
    # Create the redirect files first so Start-Process doesn't reject them.
    Remove-Item mcp.in, mcp.out, mcp.err -ErrorAction SilentlyContinue
    New-Item -ItemType File -Force -Path mcp.in  | Out-Null
    New-Item -ItemType File -Force -Path mcp.out | Out-Null
    New-Item -ItemType File -Force -Path mcp.err | Out-Null
    $Global:E2E_McpProc = Start-Process -FilePath $Global:E2E_BinCairnCli -ArgumentList 'mcp' `
        -RedirectStandardInput mcp.in -RedirectStandardOutput mcp.out `
        -RedirectStandardError mcp.err -NoNewWindow -PassThru
    # initialize
    $init = @{ jsonrpc='2.0'; id=1; method='initialize'; params=@{ protocolVersion='2025-06-18' } } | ConvertTo-Json -Compress
    Add-Content -Path mcp.in -Value $init
    Start-Sleep -Milliseconds 600
    $out = Get-Content mcp.out -Raw -ErrorAction SilentlyContinue
    if (-not $out -or $out -notmatch 'protocolVersion') {
        Write-Log "WARN: MCP initialize returned: $out"
    }
    $Global:E2E_McpId = 2
}

function Invoke-Mcp {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)] [string] $Tool,
        [hashtable] $Args = @{}
    )
    if (-not $Global:E2E_McpProc -or $Global:E2E_McpProc.HasExited) {
        Initialize-Mcp
    }
    $req = @{
        jsonrpc = '2.0'
        id = $Global:E2E_McpId
        method = 'tools/call'
        params = @{ name = $Tool; arguments = $Args }
    }
    $Global:E2E_McpId++
    Add-Content -Path mcp.in -Value ($req | ConvertTo-Json -Compress -Depth 16)
    $deadline = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 150
        $lines = Get-Content mcp.out -ErrorAction SilentlyContinue
        if ($lines -and $lines.Count -ge $Global:E2E_McpId - 1) {
            for ($i = $lines.Count - 1; $i -ge 0; $i--) {
                $line = $lines[$i]
                if ([string]::IsNullOrWhiteSpace($line)) { continue }
                try {
                    $j = $line | ConvertFrom-Json -ErrorAction Stop
                    if ($j.id -eq $Global:E2E_McpId - 1) {
                        return $j
                    }
                } catch { }
            }
        }
    }
    throw "MCP call to $Tool timed out (id=$($Global:E2E_McpId - 1))"
}

function Invoke-McpResourcesRead {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)] [string] $Uri)
    $req = @{
        jsonrpc = '2.0'
        id = $Global:E2E_McpId
        method = 'resources/read'
        params = @{ uri = $Uri }
    }
    $Global:E2E_McpId++
    Add-Content -Path mcp.in -Value ($req | ConvertTo-Json -Compress -Depth 16)
    $deadline = (Get-Date).AddSeconds(15)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 150
        $lines = Get-Content mcp.out -ErrorAction SilentlyContinue
        if ($lines) {
            for ($i = $lines.Count - 1; $i -ge 0; $i--) {
                $line = $lines[$i]
                if ([string]::IsNullOrWhiteSpace($line)) { continue }
                try {
                    $j = $line | ConvertFrom-Json -ErrorAction Stop
                    if ($j.id -eq $Global:E2E_McpId - 1) {
                        return $j
                    }
                } catch { }
            }
        }
    }
    throw "MCP resources/read of $Uri timed out"
}

function Stop-Mcp {
    if ($Global:E2E_McpProc -and -not $Global:E2E_McpProc.HasExited) {
        try { $Global:E2E_McpProc | Stop-Process -Force } catch { }
    }
    Remove-Item mcp.in, mcp.out, mcp.err -ErrorAction SilentlyContinue
}

# ----------------------------------------------------------------------
# CLI wrapper — runs `cairn-cli <args>` and returns stdout as a string.
# Uses a fresh data dir per call so we don't pollute the live container.
# ----------------------------------------------------------------------

function Invoke-CairnCli {
    [CmdletBinding()]
    param(
        [Parameter(ValueFromRemainingArguments = $true)] [string[]] $Args
    )
    $tmp = New-TemporaryFile
    try {
        & $Global:E2E_BinCairnCli --data-dir $Global:E2E_DataDir @Args 2>&1 | Tee-Object -FilePath $tmp.FullName | Out-Null
        return (Get-Content $tmp.FullName -Raw)
    } finally {
        Remove-Item $tmp -ErrorAction SilentlyContinue
    }
}

# ----------------------------------------------------------------------
# Table summary
# ----------------------------------------------------------------------

function Show-Scenario {
    param(
        [Parameter(Mandatory = $true)] [string] $Sprint,
        [Parameter(Mandatory = $true)] [string] $Name,
        [ValidateSet('pass','fail','skip')] [string] $Status = 'pass'
    )
    $Global:E2E_Rows.Add(("{0,-8}  {1,-40}  {2}" -f $Sprint, $Name, $Status.ToUpper()))
}

function Show-Summary {
    $Global:E2E_Rows.Add("")
    $Global:E2E_Rows.Add(("Total: {0}  Passed: {1}  Failed: {2}  Skipped: {3}" -f `
        $Global:E2E_Total, $Global:E2E_Passed, $Global:E2E_Failed, $Global:E2E_Skipped))
    $Global:E2E_Rows | ForEach-Object { Write-Host $_ }
}

function Exit-Summary {
    Show-Summary
    Stop-Mcp
    if ($Global:E2E_Failed -gt 0) { exit 1 } else { exit 0 }
}
