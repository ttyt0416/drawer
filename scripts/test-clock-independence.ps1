param([string]$Executable='dist/Drawer-windows-x64/Drawer.exe',[string]$EvidenceFile)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$exe=(Resolve-Path -LiteralPath (Join-Path $repo $Executable)).Path
$root=Join-Path $repo ('.verify-273-clock-'+[guid]::NewGuid().ToString())
if(Test-Path -LiteralPath $root){throw '기존 시각 시험 폴더는 보존합니다.'}
[void](New-Item -ItemType Directory -Path $root)
$hostStart=[DateTime]::UtcNow
$records=@()
foreach($year in 1960,2099){
    $run=Join-Path $root ([string]$year)
    [void](New-Item -ItemType Directory -Path $run)
    $trace=Join-Path $run 'trace.json'
    $receipt=Join-Path $run 'receipt.txt'
    & python -X utf8 (Join-Path $PSScriptRoot 'win-api-probe.py') --exe $exe --output $trace --data (Join-Path $run 'data') --fake-date "$year-01-01T00:00:00" --clock-scope image --timeout 240 -- --smoke-test $receipt
    if($LASTEXITCODE -ne 0){throw "앱 시각 $year 검증 실패; $trace"}
    $result=Get-Content -LiteralPath $trace -Raw|ConvertFrom-Json
    if(-not $result.passed -or $result.ownedProcessesRemaining -ne 0 -or @($result.networkAttempts).Count -ne 0 -or -not (Test-Path -LiteralPath $receipt)){throw "시각 $year의 완료 증거 부족"}
    $content=Get-Content -LiteralPath $receipt -Raw
    foreach($flag in 'local_inference','local_acquisition','asset_catalog','data_maintenance','mouse_painting','native_generation','atomic_save','background_save'){
        if($content -notmatch ($flag+'=true')){throw "시각 $year 기능 거절: $flag"}
    }
    $records+=@{year=$year;clockCalls=$result.clockCalls;networkAttempts=@($result.networkAttempts);ownedProcessesRemaining=$result.ownedProcessesRemaining;receiptSha256=(Get-FileHash -LiteralPath $receipt -Algorithm SHA256).Hash;traceSha256=(Get-FileHash -LiteralPath $trace -Algorithm SHA256).Hash;loadedModules=$result.loadedModules}
}
$hostEnd=[DateTime]::UtcNow
if($hostStart.Year -ne $hostEnd.Year -or ($hostEnd-$hostStart).TotalMinutes -gt 15){throw '호스트 시각 변화/시험 시간 범위를 확인하세요.'}
$summary=@{version=1;passed=$true;exeSha256=(Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash;hostClockUnchanged=$true;scope='Owned executable images and their owned child images; OS/vendor DLL clock calls pass through real time';cases=$records;limits='Finite installed-app scenario. Fresh PC/network-disconnection evidence is separate item272. Direct syscalls and unrelated services are not observed.'}
if($EvidenceFile){$destination=[IO.Path]::GetFullPath($EvidenceFile);$summary|ConvertTo-Json -Depth 12|Set-Content -LiteralPath $destination -Encoding utf8}
$summary|ConvertTo-Json -Depth 5
