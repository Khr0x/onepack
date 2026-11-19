# End-to-end test on Windows with nuget.exe and dotnet (Phase 3, ADR-019).
# nuget.exe: push, search (with and without prerelease), and install with range resolution.
# dotnet: restore with NuGet.Config credentials and consumer execution.
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

$root = (Resolve-Path "$PSScriptRoot/../..").Path
$conformance = Join-Path $root 'tests/conformance-dotnet'
$work = Join-Path ([IO.Path]::GetTempPath()) ("onepack-e2e-" + [guid]::NewGuid())
$port = 5899
$feed = 'e2e'
$base = "http://127.0.0.1:$port"
$source = "$base/nuget/$feed/v3/index.json"
$exe = Join-Path $root 'target/debug/onepackd.exe'
$server = $null

function Step($message) { Write-Host "==> $message" }

function Fail($message) {
    Write-Host "E2E FAILED: $message"
    if (Test-Path "$work/server.log") { Get-Content "$work/server.log" | Write-Host }
    throw $message
}

function Invoke-Native {
    param([string]$File, [string[]]$Arguments)
    $output = & $File @Arguments 2>&1 | Out-String
    if ($LASTEXITCODE -ne 0) { Fail "$File $($Arguments -join ' ') exited with $LASTEXITCODE`n$output" }
    return $output
}

function Onepackd([string[]]$Arguments) {
    Invoke-Native $exe ($Arguments + @('--data-dir', "$work/data"))
}

try {
    New-Item -ItemType Directory -Path $work, "$work/src", "$work/client" | Out-Null
    Copy-Item "$conformance/global.json" "$work/global.json"
    Copy-Item -Recurse "$conformance/fixtures" "$work/src/fixtures"
    Copy-Item -Recurse "$conformance/consumer" "$work/client/consumer"
    Get-ChildItem -Recurse -Directory -Path $work -Include bin, obj | Remove-Item -Recurse -Force
    Push-Location $work
    $sdk = (Invoke-Native 'dotnet' @('--version')).Trim()
    Pop-Location
    $tfm = "net$($sdk.Split('.')[0]).0"
    Step "SDK .NET $sdk (consumer: $tfm)"

    Step 'Building onepackd'
    Invoke-Native 'cargo' @('build', '--quiet', '--manifest-path', "$root/Cargo.toml", '-p', 'onepack-server') | Out-Null

    Step 'Initializing the data directory and a CI service account'
    Onepackd @('init') | Out-Null
    Onepackd @('feed', 'create', $feed) | Out-Null
    Onepackd @('principal', 'create', 'ci', '--kind', 'service') | Out-Null
    Onepackd @('grant', 'set', '--principal', 'ci', '--feed', $feed, '--role', 'publisher') | Out-Null
    $token = (& $exe token create --principal ci --expires-in-days 1 --data-dir "$work/data" 2>$null | Out-String).Trim()
    if (-not $token.StartsWith('opk_')) { Fail 'could not create the token' }
    $headers = @{ 'X-NuGet-ApiKey' = $token }

    Step "Starting onepackd at $base"
    $server = Start-Process -FilePath $exe -PassThru -NoNewWindow `
        -RedirectStandardError "$work/server.log" -RedirectStandardOutput "$work/server.out" `
        -ArgumentList @('serve', '--listen', "127.0.0.1:$port", '--data-dir', "$work/data",
                        '--public-url', $base, '--min-free-space-mib', '0')
    $ready = $false
    for ($i = 0; $i -lt 50 -and -not $ready; $i++) {
        try { Invoke-RestMethod -Uri $source -Headers $headers | Out-Null; $ready = $true }
        catch { Start-Sleep -Milliseconds 200 }
    }
    if (-not $ready) { Fail "onepackd did not respond at $source" }

    Step 'Packing fixtures with dotnet pack'
    Push-Location $work
    function Pack([string]$Project, [string[]]$Extra = @()) {
        Invoke-Native 'dotnet' (@('pack', "src/fixtures/$Project", '-c', 'Release', '-o', "$work/nupkgs", '--nologo', '-v', 'quiet') + $Extra) | Out-Null
    }
    Pack 'Onepack.Fixture.Basic'
    Pack 'Onepack.Fixture.Basic' @('-p:Version=1.1.0')
    Pack 'Onepack.Fixture.Basic' @('-p:Version=2.0.0')
    Pack 'Onepack.Fixture.Dependent'
    Pack 'Onepack.Fixture.Rich'
    Pack 'Onepack.Fixture.Ranged' @("-p:RestoreAdditionalProjectSources=$work/nupkgs")
    Pop-Location

    Step 'Downloading nuget.exe'
    $nuget = "$work/nuget.exe"
    Invoke-WebRequest -Uri 'https://dist.nuget.org/win-x86-commandline/latest/nuget.exe' -OutFile $nuget
    Write-Host "    nuget.exe $((Get-Item $nuget).VersionInfo.FileVersion)"

    $config = "$work/client/NuGet.Config"
    @"
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <packageSources>
    <clear />
    <add key="onepack" value="$source" protocolVersion="3" allowInsecureConnections="true" />
  </packageSources>
  <packageSourceCredentials>
    <onepack>
      <add key="Username" value="ci" />
      <add key="ClearTextPassword" value="$token" />
    </onepack>
  </packageSourceCredentials>
</configuration>
"@ | Set-Content -Path $config -Encoding utf8
    $env:NUGET_PACKAGES = "$work/packages"
    $env:NUGET_HTTP_CACHE_PATH = "$work/http-cache"

    Step 'Publishing with nuget.exe push'
    foreach ($pkg in Get-ChildItem "$work/nupkgs/*.nupkg") {
        Invoke-Native $nuget @('push', $pkg.FullName, '-Source', 'onepack', '-ApiKey', $token, '-ConfigFile', $config, '-NonInteractive') | Out-Null
    }

    Step 'Checking that a duplicate version is rejected (409)'
    $dup = & $nuget push "$work/nupkgs/Onepack.Fixture.Basic.1.0.0.nupkg" -Source onepack -ApiKey $token -ConfigFile $config -NonInteractive 2>&1 | Out-String
    if ($LASTEXITCODE -eq 0 -or $dup -notmatch '409') { Fail "duplicate push should fail with 409: $dup" }

    Step 'Searching with nuget.exe search (without and with prerelease)'
    # `nuget search` does not accept -ConfigFile: it reads NuGet.Config from the current directory.
    Push-Location "$work/client"
    $stable = Invoke-Native $nuget @('search', 'Onepack.Fixture', '-Source', 'onepack', '-NonInteractive')
    foreach ($id in 'Onepack.Fixture.Basic', 'Onepack.Fixture.Dependent', 'Onepack.Fixture.Ranged') {
        if ($stable -notmatch [regex]::Escape($id)) { Fail "search does not show $id`n$stable" }
    }
    if ($stable -match 'Onepack\.Fixture\.Rich') { Fail "search without prerelease shows Rich`n$stable" }
    $pre = Invoke-Native $nuget @('search', 'Onepack.Fixture', '-PreRelease', '-Source', 'onepack', '-NonInteractive')
    Pop-Location
    if ($pre -notmatch 'Onepack\.Fixture\.Rich') { Fail "search with prerelease does not show Rich`n$pre" }

    Step 'Installing with nuget.exe install (range resolution)'
    Invoke-Native $nuget @('install', 'Onepack.Fixture.Ranged', '-Version', '1.0.0', '-Source', 'onepack', '-ConfigFile', $config,
                           '-OutputDirectory', "$work/installed", '-NonInteractive') | Out-Null
    if (-not (Test-Path "$work/installed/Onepack.Fixture.Basic.1.1.0")) {
        Fail "nuget install did not resolve Basic 1.1.0: $((Get-ChildItem "$work/installed").Name -join ', ')"
    }

    Step 'Restoring and running the consumer with dotnet'
    Push-Location "$work/client/consumer"
    Invoke-Native 'dotnet' @('restore', '--configfile', $config, '--nologo', "-p:ConsumerTargetFramework=$tfm") | Out-Null
    $output = (Invoke-Native 'dotnet' @('run', '--no-restore', '--nologo', "-p:ConsumerTargetFramework=$tfm")).Trim() -replace "`r", ''
    Pop-Location
    $expected = "dependent -> hello from onepack fixture`nranged -> hello from onepack fixture`nrich"
    if ($output -ne $expected) { Fail "unexpected output: '$output'" }

    Write-Host 'Windows E2E OK: nuget.exe push/409/search/install and dotnet restore/run.'
}
finally {
    if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force }
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
