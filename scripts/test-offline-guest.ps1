param([Parameter(Mandatory=$true)][string]$OwnerToken,[switch]$ShutdownGuest)
$ErrorActionPreference='Stop'
$evidence='C:\DrawerEvidence'
$ownerPath='C:\DrawerTask\owner.json'
$verifiedGuest=$false
$result=@{version=1;passed=$false;verifiedFreshOffline=$false;error=$null}
try{
    if($OwnerToken -notmatch '^[a-fA-F0-9-]{36}$' -or $env:USERNAME -ne 'WDAGUtilityAccount'){throw '이 스크립트는 소유한 새 Windows Sandbox 게스트 전용입니다.'}
    $system=Get-CimInstance Win32_ComputerSystem
    if(-not $system.HypervisorPresent -or $system.Model -notmatch 'Virtual'){throw 'Windows 가상 게스트가 아닙니다.'}
    $owner=Get-Content -LiteralPath $ownerPath -Raw|ConvertFrom-Json
    if($owner.token -ne $OwnerToken){throw '소유 표식이 다릅니다.'}
    $verifiedGuest=$true
    if($env:USERDOMAIN -ne $env:COMPUTERNAME){throw '로컬 게스트 계정이 아닙니다.'}
    $tools=@('rustc','cargo','node','python')|ForEach-Object{Get-Command $_ -ErrorAction SilentlyContinue}|Where-Object{if($_.Source -and (Test-Path -LiteralPath $_.Source -PathType Leaf)){$m=Get-Item -LiteralPath $_.Source;($m.Length -gt 0) -and -not ($m.Attributes -band [IO.FileAttributes]::ReparsePoint)}}
    if($tools){throw '개발 런타임이 발견되어 새 OS 기준 검증을 거절합니다.'}
    $adapters=@(Get-NetAdapter -IncludeHidden|Where-Object Status -eq 'Up')
    if($adapters.Count -ne 0){throw '활성 네트워크 어댑터가 있습니다. 호스트 네트워크는 변경하지 않습니다.'}
    Clear-DnsClientCache
    $resolved=$false
    try{Resolve-DnsName 'www.microsoft.com' -DnsOnly -NoHostsFile -QuickTimeout -ErrorAction Stop|Out-Null;$resolved=$true}catch{}
    if($resolved){throw '서버 이름 해석이 성공하여 네트워크 차단 기준을 충족하지 않습니다.'}
    $program=Join-Path $env:LOCALAPPDATA ('Programs\Drawer-'+$OwnerToken)
    $data=Join-Path $env:LOCALAPPDATA ('DrawerValidation-'+$OwnerToken)
    if((Test-Path -LiteralPath $program) -or (Test-Path -LiteralPath $data)){throw '빈 설치/캐시 경로가 아닙니다.'}
    $package='C:\DrawerPackage'
    $hash=(Get-FileHash -LiteralPath (Join-Path $package 'Drawer.exe') -Algorithm SHA256).Hash
    if($hash -ne $owner.expectedExeSha256){throw '패키지 무결성 불일치'}
    & (Join-Path $package 'Install.ps1') -InstallDir $program -ExpectedManifestSha256 (Get-FileHash -LiteralPath (Join-Path $package 'PACKAGE.json')).Hash -AllowUnsigned
    $env:DRAWER_DATA_DIR=$data
    $env:DRAWER_SMOKE_DIAGNOSTICS=$evidence
    $env:PATH=Join-Path $env:WINDIR 'System32'
    foreach($run in 1,2){
        $receipt=Join-Path $evidence ('launch-'+$run+'.txt')
        $errors=Join-Path $evidence ('errors-'+$run+'.txt')
        $child=Start-Process -FilePath (Join-Path $program 'Drawer.exe') -ArgumentList @('--smoke-test',('"'+$receipt+'"')) -WindowStyle Hidden -RedirectStandardError $errors -PassThru
        if(-not $child.WaitForExit(180000)){$child.Kill();$child.WaitForExit();throw '게스트 앱 시험 시간 초과'}
        if($child.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $receipt)){throw "게스트 앱 실행 $run 실패"}
        $text=Get-Content -LiteralPath $receipt -Raw
        foreach($flag in 'mouse_painting','save_as_export','native_generation','atomic_save','saved_state','background_save','local_help','local_color_profile','local_inference'){
            if($text -notmatch ($flag+'=true')){throw "게스트 핵심 상태 실패: $flag"}
        }
    }
    $result.passed=$true;$result.verifiedFreshOffline=$true;$result.exeSha256=$hash
    $result.emptyCacheAtStart=$true;$result.localAccount=$true;$result.developmentRuntimesAbsent=$true;$result.activeNetworkAdapters=0;$result.dnsResolutionFailed=$true;$result.launches=2
    $result.windowsBuild=[Environment]::OSVersion.Version.ToString()
}catch{$result.error=$_.Exception.Message}
finally{
    if($verifiedGuest){$result|ConvertTo-Json -Depth 5|Set-Content -LiteralPath (Join-Path $evidence 'offline-result.json') -Encoding utf8;if($ShutdownGuest){& shutdown.exe /s /t 5 /f|Out-Null}}
}
if(-not $result.passed){exit 1}
