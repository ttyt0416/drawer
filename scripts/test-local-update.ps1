param([string]$Evidence='docs/process/local-update-evidence.json')
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$base=Join-Path $repo ('.verify-277-update-'+[guid]::NewGuid().ToString('N'))
$package=Join-Path $repo 'dist/Drawer-windows-x64';$installer=Join-Path $package 'Install.ps1'
$pin=(Get-FileHash -LiteralPath (Join-Path $package 'PACKAGE.json')).Hash
$program=Join-Path $base 'program';$data=Join-Path $base 'userdata'
$oldData=$env:DRAWER_DATA_DIR;$oldDiag=$env:DRAWER_SMOKE_DIAGNOSTICS;$child=$null
function Must-Refuse([scriptblock]$Action) { $refused=$false;try{& $Action|Out-Null}catch{$refused=$true;Write-Output ('Expected refusal: '+$_.Exception.Message)};if(-not $refused){throw '거절해야 하는 설치를 허용했습니다.'} }
function Files([string]$Path) {$map=@{};Get-ChildItem -LiteralPath $Path -Recurse -File -Force|ForEach-Object{$map[[IO.Path]::GetRelativePath($Path,$_.FullName)]=(Get-FileHash -LiteralPath $_.FullName).Hash};return $map}
function Assert-Files($Before,[string]$Path) {$after=Files $Path;if($Before.Count -ne $after.Count){throw '실패 후 파일 수 변경'};foreach($name in $Before.Keys){if($Before[$name] -ne $after[$name]){throw '실패 후 기존 바이트 변경'}}}
try {
    [void](New-Item -ItemType Directory -Path $base)
    Must-Refuse {& $installer -InstallDir $program -ExpectedManifestSha256 $pin}
    if(Test-Path -LiteralPath $program){throw '서명 거절 후 설치 폴더 생성'}
    Must-Refuse {& $installer -InstallDir $program -ExpectedManifestSha256 ('0'*64) -AllowUnsigned}
    & $installer -InstallDir $program -ExpectedManifestSha256 $pin -AllowUnsigned
    [void](New-Item -ItemType Directory -Path (Join-Path $data 'settings'))
    [IO.File]::WriteAllText((Join-Path $data 'settings/resources.json'),'{"version":1}')
    [IO.File]::WriteAllText((Join-Path $data 'settings/preferences.txt'),('version=1'+[char]10+'brush=Pencil'+[char]10))
    [IO.File]::WriteAllText((Join-Path $program 'user-note.txt'),'keep user extra')
    $ps5=Join-Path $env:SystemRoot 'System32/WindowsPowerShell/v1.0/powershell.exe'
    $ps5Install=Join-Path $base 'windows-powershell'
    foreach($mode in 'install','reinstall','uninstall'){
        $arguments=if($mode -eq 'uninstall'){@('-NoProfile','-ExecutionPolicy','Bypass','-File',('"'+(Join-Path $package 'Uninstall.ps1')+'"'),'-InstallDir',('"'+$ps5Install+'"'))}else{@('-NoProfile','-ExecutionPolicy','Bypass','-File',('"'+$installer+'"'),'-InstallDir',('"'+$ps5Install+'"'),'-ExpectedManifestSha256',$pin,'-AllowUnsigned')}
        $ps=Start-Process -FilePath $ps5 -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait -RedirectStandardOutput (Join-Path $base ($mode+'-ps5.out')) -RedirectStandardError (Join-Path $base ($mode+'-ps5.err'))
        if($ps.ExitCode -ne 0){Get-Content -LiteralPath (Join-Path $base ($mode+'-ps5.err'));throw 'Windows 기본 PowerShell 설치/재설치/제거 실패'}
    }
    $dataBefore=Files $data
    $marker=Join-Path $program '.drawer-install.json'
    $record=Get-Content -LiteralPath $marker -Raw|ConvertFrom-Json
    $record.version='0.0.1';$record|ConvertTo-Json -Depth 5|Set-Content -LiteralPath $marker -Encoding utf8
    $before=Files $program
    $bad=Join-Path $base 'tampered';Copy-Item -LiteralPath $package -Destination $bad -Recurse
    Add-Content -LiteralPath (Join-Path $bad 'HELP.md') -Value 'tamper'
    Must-Refuse {& (Join-Path $bad 'Install.ps1') -InstallDir $program -ExpectedManifestSha256 $pin -AllowUnsigned}
    Assert-Files $before $program
    Copy-Item -LiteralPath (Join-Path $package 'HELP.md') -Destination (Join-Path $bad 'HELP.md') -Force
    $badExe=Join-Path $bad 'Drawer.exe';$bytes=[IO.File]::ReadAllBytes($badExe)
    $pe=[BitConverter]::ToUInt32($bytes,60);$at=$pe+24+112+32
    $offset=([int64][Math]::Ceiling($bytes.Length/8.0))*8
    $corrupt=[byte[]]::new($offset+16);$bytes.CopyTo($corrupt,0)
    [BitConverter]::GetBytes([uint32]$offset).CopyTo($corrupt,$at)
    [BitConverter]::GetBytes([uint32]16).CopyTo($corrupt,$at+4)
    [BitConverter]::GetBytes([uint32]16).CopyTo($corrupt,$offset)
    [BitConverter]::GetBytes([uint16]0x200).CopyTo($corrupt,$offset+4)
    [BitConverter]::GetBytes([uint16]2).CopyTo($corrupt,$offset+6)
    [IO.File]::WriteAllBytes($badExe,$corrupt)
    $badManifest=Join-Path $bad 'PACKAGE.json';$meta=Get-Content -LiteralPath $badManifest -Raw|ConvertFrom-Json
    foreach($entry in $meta.files){if($entry.name -eq 'Drawer.exe'){$entry.bytes=$corrupt.Length;$entry.sha256=(Get-FileHash -LiteralPath $badExe).Hash.ToLowerInvariant()}}
    $meta|ConvertTo-Json -Depth 5|Set-Content -LiteralPath $badManifest -Encoding utf8
    $badPin=(Get-FileHash -LiteralPath $badManifest).Hash
    Must-Refuse {& (Join-Path $bad 'Install.ps1') -InstallDir $program -ExpectedManifestSha256 $badPin -AllowUnsigned}
    Assert-Files $before $program
    # Inject failure at publication, after the old installation was really renamed.
    $promotionProbe=[pscustomobject]@{rejected=$false}
    function Move-Item {
        param([string]$LiteralPath,[string]$Destination)
        if(-not $promotionProbe.rejected -and (Split-Path -Leaf $LiteralPath).StartsWith('.Drawer-stage-') -and $Destination -eq $program){$promotionProbe.rejected=$true;throw 'deterministic promotion failure'}
        Microsoft.PowerShell.Management\Move-Item -LiteralPath $LiteralPath -Destination $Destination
    }
    try {Must-Refuse {& $installer -InstallDir $program -ExpectedManifestSha256 $pin -AllowUnsigned}}
    finally {Remove-Item -LiteralPath Function:\Move-Item}
    if(-not $promotionProbe.rejected){throw '게시 실패 주입 경계 미실행'}
    Assert-Files $before $program;Assert-Files $dataBefore $data
    & $installer -InstallDir $program -ExpectedManifestSha256 $pin -AllowUnsigned
    Assert-Files $dataBefore $data
    if((Get-Content -LiteralPath (Join-Path $program 'user-note.txt') -Raw) -ne 'keep user extra'){throw '추가 파일 보존 실패'}
    $previous=@(Get-ChildItem -LiteralPath $base -Directory -Force|Where-Object {$_.Name.StartsWith('.Drawer-previous-') -and (Get-Content -LiteralPath (Join-Path $_.FullName '.drawer-install.json') -Raw|ConvertFrom-Json).installRoot -eq $program})
    if($previous.Count -ne 1 -or (Get-FileHash -LiteralPath (Join-Path $previous[0].FullName 'Drawer.exe')).Hash -ne $before['Drawer.exe']){throw '이전 정상 앱 보존 실패'}
    $record=Get-Content -LiteralPath $marker -Raw|ConvertFrom-Json;$record.version='9.0.0';$record|ConvertTo-Json -Depth 5|Set-Content -LiteralPath $marker -Encoding utf8
    $future=Files $program
    Must-Refuse {& $installer -InstallDir $program -ExpectedManifestSha256 $pin -AllowUnsigned}
    Assert-Files $future $program
    & $installer -InstallDir $program -ExpectedManifestSha256 $pin -AllowUnsigned -AllowDowngrade
    Assert-Files $dataBefore $data
    $env:DRAWER_DATA_DIR=$data;$env:DRAWER_SMOKE_DIAGNOSTICS=$base
    $receipt=Join-Path $base 'receipt.txt'
    $child=Start-Process -FilePath (Join-Path $program 'Drawer.exe') -ArgumentList @('--smoke-test',('"'+$receipt+'"')) -WorkingDirectory $program -WindowStyle Hidden -PassThru
    if(-not $child.WaitForExit(90000) -or $child.ExitCode -ne 0){throw '이전 설정으로 업데이트 앱 실행 실패'}
    $text=Get-Content -LiteralPath $receipt -Raw
    foreach($flag in 'native_generation=true','resource_settings=true','background_save=true','data_maintenance=true','saved_state=true'){if(-not $text.Contains($flag)){throw '업데이트 검증 누락'}}
    [ordered]@{version=1;executableSha256=(Get-FileHash -LiteralPath (Join-Path $program 'Drawer.exe')).Hash.ToLowerInvariant();manifestSha256=$pin.ToLowerInvariant();packageFiles=13;unsignedDefaultRefused=$true;incorrectManifestRefused=$true;tamperedFileRefused=$true;invalidEmbeddedSignatureRefused=$true;promotionFailureRollback=$true;promotionFailure='deterministic operation-boundary injection after real old-root rename';previousApplicationRetained=$true;extraFilesPreserved=$true;settingsBytesPreservedByInstaller=$true;olderSettingsStarted=$true;windowsPowerShell5InstallReinstallUninstall=$true;implicitDowngradeRefused=$true;explicitDowngradePassed=$true;trustFlags='WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_NONE';limitations='No publisher certificate supplied; signed-release success not claimed. Current host, not fresh-PC272.'}|ConvertTo-Json -Depth 5|Set-Content -LiteralPath (Join-Path $repo $Evidence) -Encoding utf8
    Write-Output '수동 업데이트/해시·서명 거절/실패 복구/설정·추가 파일 보존/이전 설정 실행 통과'
} finally {
    if($child -and -not $child.HasExited){$child.Kill();$child.WaitForExit()}
    $env:DRAWER_DATA_DIR=$oldData;$env:DRAWER_SMOKE_DIAGNOSTICS=$oldDiag
    if(Test-Path -LiteralPath $base){
        if(-not ([IO.Path]::GetFullPath($base)).StartsWith($repo+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw '시험 삭제 범위 오류'}
        foreach($entry in @(Get-Item -LiteralPath $base)+@(Get-ChildItem -LiteralPath $base -Recurse -Force)){if($entry.Attributes -band [IO.FileAttributes]::ReparsePoint){throw '시험 링크 보존'}}
        Remove-Item -LiteralPath $base -Recurse -Force
    }
}
