# Native Windows Credential Manager acceptance: login, use and logout.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$root = (Resolve-Path "$PSScriptRoot/..").Path
$port = 5912
$base = "http://127.0.0.1:$port"
$context = 'local'
$work = Join-Path ([IO.Path]::GetTempPath()) ("onepack-keychain-" + [guid]::NewGuid())
$server = $null
$cli = $null

function Fail($message) {
    Write-Host "WINDOWS KEYCHAIN E2E FAILED: $message"
    if (Test-Path "$work/server.log") { Get-Content "$work/server.log" | Write-Host }
    throw $message
}

try {
    if ($env:ONEPACK_BIN_DIR) {
        $bin = (Resolve-Path $env:ONEPACK_BIN_DIR).Path
    } else {
        Write-Host '==> building onepack and onepackd'
        & cargo build --quiet --manifest-path "$root/Cargo.toml" -p onepack-server -p onepack-cli
        if ($LASTEXITCODE -ne 0) { Fail "cargo build exited with $LASTEXITCODE" }
        $bin = Join-Path $root 'target/debug'
    }
    $cli = Join-Path $bin 'onepack.exe'
    $daemon = Join-Path $bin 'onepackd.exe'

    New-Item -ItemType Directory -Path $work -Force | Out-Null
    $env:ONEPACK_CONFIG_DIR = Join-Path $work 'config'
    $env:ONEPACK_NO_INPUT = '1'
    Remove-Item Env:ONEPACK_TOKEN, Env:ONEPACK_KEYRING, Env:ONEPACK_CA_CERT, Env:ONEPACK_URL, Env:ONEPACK_CONTEXT -ErrorAction SilentlyContinue

    Write-Host '==> starting onepackd'
    & $daemon init --data-dir "$work/data" | Out-Null
    $token = (Get-Content "$work/data/initial-admin-token" -Raw).Trim()
    $server = Start-Process -FilePath $daemon -PassThru -NoNewWindow `
        -RedirectStandardError "$work/server.log" -RedirectStandardOutput "$work/server.out" `
        -ArgumentList @('serve', '--listen', "127.0.0.1:$port", '--data-dir', "$work/data",
                        '--public-url', $base, '--min-free-space-mib', '0')
    $ready = $false
    for ($i = 0; $i -lt 50 -and -not $ready; $i++) {
        try { Invoke-WebRequest -Uri "$base/readyz" -UseBasicParsing | Out-Null; $ready = $true }
        catch { Start-Sleep -Milliseconds 200 }
    }
    if (-not $ready) { Fail 'onepackd did not respond at readyz' }

    Write-Host '==> login, use and logout with Windows Credential Manager'
    & $cli context add $context --url $base --use | Out-Null
    $token | & $cli login --token-stdin | Out-Null
    if ($LASTEXITCODE -ne 0) { Fail "login exited with $LASTEXITCODE" }
    $me = & $cli whoami --json | ConvertFrom-Json
    if ($me.principal -ne 'admin') { Fail "whoami returned $($me.principal)" }
    $doctor = & $cli doctor --json | ConvertFrom-Json
    $check = $doctor.checks | Where-Object { $_.name -eq 'credentials' }
    if (-not $check -or $check.status -ne 'ok' -or $check.message -notmatch 'system keychain') {
        Fail "credentials check is not using the system keychain: $check"
    }
    $secret = ($token -split '_')[-1]
    Get-ChildItem -Path $env:ONEPACK_CONFIG_DIR -Recurse -File -ErrorAction SilentlyContinue |
        ForEach-Object {
            if (Select-String -Path $_.FullName -SimpleMatch $secret -Quiet) {
                Fail 'the token was written to the CLI configuration'
            }
        }

    & $cli logout | Out-Null
    & $cli whoami 2>$null | Out-Null
    $code = $LASTEXITCODE
    if ($code -ne 3) { Fail "whoami after logout exited $code, expected 3" }

    Write-Host 'WINDOWS KEYCHAIN E2E OK: login, whoami, doctor and logout with Credential Manager.'
}
finally {
    if ($cli -and (Test-Path $cli)) {
        & $cli --context $context logout 2>$null | Out-Null
    }
    if ($server) { Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue }
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
