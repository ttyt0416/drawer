param([string]$Evidence='docs/process/local-storage-evidence.json')
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$base=Join-Path $repo ('.verify-275-storage-'+[guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $base)
$trace=Join-Path $base 'trace.json'; $receipt=Join-Path $base 'receipt.txt'; $data=Join-Path $base 'userdata'
python -X utf8 -B (Join-Path $PSScriptRoot 'win-api-probe.py') --exe (Join-Path $repo 'dist/Drawer-windows-x64/Drawer.exe') --output $trace --data $data --timeout 240 -- --smoke-test $receipt
if($LASTEXITCODE -ne 0){throw '로컬 저장/통신 관측 실패'}
$result=Get-Content -LiteralPath $trace -Raw|ConvertFrom-Json
if(-not $result.passed -or @($result.networkAttempts).Count -ne 0 -or $result.ownedProcessesRemaining -ne 0){throw '외부 통신 시도 또는 프로세스 잔류'}
$text=Get-Content -LiteralPath $receipt -Raw
foreach($flag in 'asset_catalog=true','work_library=true','work_time=true','file_browser=true','data_maintenance=true','local_recovery=true','background_save=true','image_materials=true','saved_state=true'){if(-not $text.Contains($flag)){throw '로컬 저장 기능 시험 누락'}}
$inventory=@(Get-ChildItem -LiteralPath $data -Recurse -File -Force|Sort-Object FullName|ForEach-Object {
    if($_.Attributes -band [IO.FileAttributes]::ReparsePoint){throw '시험 저장소 링크'}
    [ordered]@{relativePath=[IO.Path]::GetRelativePath($data,$_.FullName).Replace('\','/');bytes=$_.Length;sha256=(Get-FileHash -LiteralPath $_.FullName).Hash.ToLowerInvariant()}
})
if($inventory.Count -lt 20){throw '저장 목록이 비어 있음'}
foreach($key in 'settings/','materials/'){if(-not @($inventory|Where-Object {$_.relativePath.StartsWith($key)}).Count){throw '설정/소재 저장 누락'}}
[ordered]@{version=1;executableSha256=$result.exeSha256;networkAttempts=0;ownedProcessesRemaining=0;networkApiCount=$result.networkApiCount;storage='fresh synthetic local user directory';writes=$inventory;diagnostics='Explicit smoke harness only; API trace omits arguments/pixels/user paths';limitations=$result.limitations}|ConvertTo-Json -Depth 8|Set-Content -LiteralPath (Join-Path $repo $Evidence) -Encoding utf8
Write-Output ('로컬 저장 파일 '+$inventory.Count+'개; 관측 통신 시도0; 자체 프로세스 잔류0')
