param([switch]$CheckOnly,[string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$sandbox=Join-Path $env:WINDIR 'System32/WindowsSandbox.exe'
$package=Join-Path $repo 'dist/Drawer-windows-x64'
$ready=(Test-Path -LiteralPath $sandbox -PathType Leaf)
if($CheckOnly){@{sandboxAvailable=$ready;packageAvailable=(Test-Path -LiteralPath (Join-Path $package 'Drawer.exe'));verifiedFreshOffline=$false}|ConvertTo-Json;return}
if(-not $ready){throw 'Windows Sandbox가 없습니다. 새 Windows VM에서 게스트 검증을 실행할 환경이 필요합니다. 현재 PC 시험을 새 PC 검증으로 간주하지 않습니다.'}
if(Get-Process 'WindowsSandbox*' -ErrorAction SilentlyContinue){throw '기존 Sandbox는 보존합니다. 기존 세션을 종료한 뒤 다시 실행하세요.'}
if(-not $EvidenceDirectory){throw '증거를 보관할 새 로컬 EvidenceDirectory를 지정하세요.'}
$evidence=[IO.Path]::GetFullPath($EvidenceDirectory)
if(Test-Path -LiteralPath $evidence){throw '기존 증거 폴더는 보존합니다.'}
if($evidence.StartsWith('\\') -or $evidence -match '^[a-z]+://'){throw '로컬 증거 경로가 필요합니다.'}
[void](New-Item -ItemType Directory -Path $evidence)
$token=[guid]::NewGuid().ToString()
$harness=Join-Path $evidence 'harness'
[void](New-Item -ItemType Directory -Path $harness)
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'test-offline-guest.ps1') -Destination (Join-Path $harness 'offline-guest.ps1')
@{token=$token;expectedExeSha256=(Get-FileHash -LiteralPath (Join-Path $package 'Drawer.exe') -Algorithm SHA256).Hash}|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $harness 'owner.json') -Encoding utf8
$escape={param($value)[Security.SecurityElement]::Escape($value)}
$packageXml=&$escape $package
$harnessXml=&$escape $harness
$evidenceXml=&$escape $evidence
$configuration=@"
<Configuration><Networking>Disable</Networking><VGpu>Enable</VGpu><MemoryInMB>4096</MemoryInMB><ClipboardRedirection>Disable</ClipboardRedirection><MappedFolders>
<MappedFolder><HostFolder>$packageXml</HostFolder><SandboxFolder>C:\DrawerPackage</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
<MappedFolder><HostFolder>$harnessXml</HostFolder><SandboxFolder>C:\DrawerTask</SandboxFolder><ReadOnly>true</ReadOnly></MappedFolder>
<MappedFolder><HostFolder>$evidenceXml</HostFolder><SandboxFolder>C:\DrawerEvidence</SandboxFolder><ReadOnly>false</ReadOnly></MappedFolder>
</MappedFolders><LogonCommand><Command>powershell.exe -NoProfile -ExecutionPolicy Bypass -File C:\DrawerTask\offline-guest.ps1 -OwnerToken $token -ShutdownGuest</Command></LogonCommand></Configuration>
"@
$config=Join-Path $evidence 'offline.wsb'
[IO.File]::WriteAllText($config,$configuration,[Text.UTF8Encoding]::new($false))
$receipt=Join-Path $evidence 'offline-result.json'
$process=Start-Process -FilePath $sandbox -ArgumentList ('"'+$config+'"') -WindowStyle Hidden -PassThru
$timer=[Diagnostics.Stopwatch]::StartNew()
while(-not (Test-Path -LiteralPath $receipt)){if($timer.Elapsed.TotalSeconds -gt 300){throw 'VM 검증 시간 초과. 증거와 해당 Sandbox를 확인하세요. 기존 VM/호스트 네트워크/시각은 변경하지 않았습니다.'};Start-Sleep -Seconds 1}
$result=Get-Content -LiteralPath $receipt -Raw|ConvertFrom-Json
if(-not $result.passed){throw ('새 Windows 오프라인 검증 실패: '+$result.error)}
if($result.exeSha256 -ne (Get-FileHash -LiteralPath (Join-Path $package 'Drawer.exe') -Algorithm SHA256).Hash){throw '게스트와 호스트 패키지가 다릅니다.'}
Write-Output "실제 새 Sandbox 오프라인 설치/재실행 검증 통과: $receipt"
# Evidence is an explicitly chosen permanent artifact, never silently removed.
