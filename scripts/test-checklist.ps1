param([ValidateRange(1,309)][int]$Item,[string]$Evidence,[ValidateRange(5,900)][int]$TimeoutSeconds=180)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$tag='{0:000}' -f $Item
$base=Join-Path $repo ('.verify-'+$tag+'-check-'+[guid]::NewGuid().ToString('N'))
$errors=$base+'.stderr.txt'
$exe=Join-Path $repo 'dist/Drawer-windows-x64/Drawer.exe'
$package=Get-Content -LiteralPath (Join-Path $repo 'dist/Drawer-windows-x64/PACKAGE.json') -Raw -Encoding utf8 | ConvertFrom-Json
$entry=@($package.files|Where-Object name -eq 'Drawer.exe')
$sha=(Get-FileHash -LiteralPath $exe).Hash.ToLowerInvariant()
if($entry.Count -ne 1 -or $entry[0].sha256 -ne $sha -or $entry[0].bytes -ne (Get-Item -LiteralPath $exe).Length){throw 'Packaged executable does not match its manifest'}
if(-not $Evidence){$Evidence='docs/process/check-'+$tag+'-evidence.json'}
$evidencePath=[IO.Path]::GetFullPath((Join-Path $repo $Evidence))
if(-not $evidencePath.StartsWith($repo+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw 'Evidence outside checkout'}
$oldData=$env:DRAWER_DATA_DIR;$oldPath=$env:PATH;$process=$null
try {
    if(Test-Path -LiteralPath $base){throw 'Verification directory must be new'}
    $env:DRAWER_DATA_DIR=Join-Path $base 'data'
    $env:PATH=Join-Path $env:SystemRoot 'System32'
    $process=Start-Process -FilePath $exe -ArgumentList @('--verify-item',"$Item",('"'+$base+'"')) -WindowStyle Hidden -RedirectStandardError $errors -PassThru
    if(-not $process.WaitForExit($TimeoutSeconds*1000)){$process.Kill();$process.WaitForExit();throw 'Owned verification timed out'}
    if($process.ExitCode -ne 0){if(Test-Path -LiteralPath $errors){Get-Content -LiteralPath $errors -Encoding utf8};throw 'Owned verification failed'}
    $receipt=Join-Path $base 'receipt.json'
    if((Get-Item -LiteralPath $receipt).Length -gt 8MB){throw 'Verification receipt exceeds8MiB'}
    $report=Get-Content -LiteralPath $receipt -Raw -Encoding utf8|ConvertFrom-Json
    if($report.format -ne 'Drawer checklist acceptance' -or $report.version -ne 1 -or $report.item -ne $Item -or $report.passed -ne $true -or $report.physicalDeviceValidated -ne $false -or $report.nativeVersion -ne $package.nativeMax){throw 'Verification receipt identity mismatch'}
    $result=[ordered]@{version=1;item=$Item;executableSha256=$sha;receiptSha256=(Get-FileHash -LiteralPath $receipt).Hash.ToLowerInvariant();result=$report}
    [IO.File]::WriteAllText($evidencePath,($result|ConvertTo-Json -Depth 100)+"`n",[Text.UTF8Encoding]::new($false))
    Write-Output "Checklist $Item acceptance passed; physical device validation=false; evidence=$Evidence"
} finally {
    if($process -and -not $process.HasExited){$process.Kill();$process.WaitForExit()}
    $env:DRAWER_DATA_DIR=$oldData;$env:PATH=$oldPath
    $absolute=[IO.Path]::GetFullPath($base)
    if(-not $absolute.StartsWith($repo+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($absolute) -notmatch ('^\.verify-'+$tag+'-check-[a-f0-9]{32}$')){throw 'Temporary cleanup outside owned checkout'}
    if(Test-Path -LiteralPath $absolute){
        foreach($p in @(Get-Item -LiteralPath $absolute -Force)+@(Get-ChildItem -LiteralPath $absolute -Recurse -Force)){if($p.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Unexpected verification link preserved'}}
        $owner=Get-Content -LiteralPath (Join-Path $absolute 'owner.json') -Raw -Encoding utf8|ConvertFrom-Json
        if($owner.format -ne 'Drawer owned verification' -or $owner.version -ne 1 -or $owner.item -ne $Item -or $owner.pid -ne $process.Id){throw 'Unknown verification directory preserved'}
        Remove-Item -LiteralPath $absolute -Recurse -Force
    }
    if(Test-Path -LiteralPath $errors){if((Get-Item -LiteralPath $errors).Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Unexpected error log link preserved'};Remove-Item -LiteralPath $errors -Force}
}
