param([string]$Evidence='docs/process/installed-assets-evidence.json')
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$base=Join-Path $repo ('.verify-274-assets-'+[guid]::NewGuid().ToString('N'))
$program=Join-Path $base 'program'; $data=Join-Path $base 'userdata'
$package=Join-Path $repo 'dist/Drawer-windows-x64'
$oldData=$env:DRAWER_DATA_DIR; $oldDiagnostics=$env:DRAWER_SMOKE_DIAGNOSTICS; $oldPath=$env:PATH
$acl=$null; $child=$null
try {
    [void](New-Item -ItemType Directory -Path $base)
    & (Join-Path $package 'Install.ps1') -InstallDir $program
    $before=@{}; Get-ChildItem -LiteralPath $program -File -Force | ForEach-Object { $before[$_.Name]=(Get-FileHash -LiteralPath $_.FullName).Hash }
    if($before.Count -ne 13){throw '설치 파일12개와 소유 기록 누락'}
    $acl=Get-Acl -LiteralPath $program
    $locked=Get-Acl -LiteralPath $program
    $deny=[Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.WindowsIdentity]::GetCurrent().User,[Security.AccessControl.FileSystemRights]::Write,[Security.AccessControl.InheritanceFlags]'ContainerInherit,ObjectInherit',[Security.AccessControl.PropagationFlags]::None,[Security.AccessControl.AccessControlType]::Deny)
    [void]$locked.AddAccessRule($deny); Set-Acl -LiteralPath $program -AclObject $locked
    $denied=$false
    try { [IO.File]::WriteAllText((Join-Path $program 'forbidden.txt'),'deny probe') } catch [UnauthorizedAccessException] { $denied=$true }
    if(-not $denied){throw '설치 폴더 쓰기 거부 대조군 실패'}
    $env:DRAWER_DATA_DIR=$data; $env:DRAWER_SMOKE_DIAGNOSTICS=$base
    $env:PATH=Join-Path $env:SystemRoot 'System32'
    $receipt=Join-Path $base 'receipt.txt'
    $child=Start-Process -FilePath (Join-Path $program 'Drawer.exe') -WorkingDirectory $program -ArgumentList @('--smoke-test',('"'+$receipt+'"')) -WindowStyle Hidden -PassThru
    if(-not $child.WaitForExit(90000)){throw '읽기전용 설치 실행 시간 초과'}
    if($child.ExitCode -ne 0){throw '읽기전용 설치 실행 실패'}
    $text=Get-Content -LiteralPath $receipt -Raw
    $flags=@('basic_brushes=true','embedded_tips=7','cursor_assets=true','local_help=true','local_help_licenses=true','local_korean_font=true','brush_presets=true','image_materials=true','local_recovery=true','saved_state=true','background_save=true')
    foreach($flag in $flags){if(-not $text.Contains($flag)){throw "자원 기능 영수증 누락: $flag"}}
    $after=@(Get-ChildItem -LiteralPath $program -Force)
    if($after.Count -ne $before.Count){throw '설치 폴더 쓰기 발생'}
    foreach($file in $after){if(-not $before.ContainsKey($file.Name) -or (Get-FileHash -LiteralPath $file.FullName).Hash -ne $before[$file.Name]){throw '설치 파일 변경'}}
    foreach($path in 'settings/preferences.txt','materials/references/reference-1.png'){if(-not (Test-Path -LiteralPath (Join-Path $data $path) -PathType Leaf)){throw '사용자 저장소 분리 실패'}}
    $report=[ordered]@{version=1;executableSha256=$before['Drawer.exe'].ToLowerInvariant();packageFiles=12;installedFilesIncludingOwnership=13;actualWriteDenied=$denied;programBytesUnchanged=$true;separateUserData=$true;systemOnlyPath=$true;functionalChecks=$flags;optionalKoreanFont='Windows installed Malgun; embedded four fonts remain available without it';limitations='Current Windows host; not the fresh offline environment of item272'}
    $report|ConvertTo-Json -Depth 6|Set-Content -LiteralPath (Join-Path $repo $Evidence) -Encoding utf8
    Write-Output '읽기전용 설치 자원 검증 통과'
} finally {
    if($child -and -not $child.HasExited){$child.Kill();$child.WaitForExit()}
    $env:DRAWER_DATA_DIR=$oldData; $env:DRAWER_SMOKE_DIAGNOSTICS=$oldDiagnostics; $env:PATH=$oldPath
    if($acl){Set-Acl -LiteralPath $program -AclObject $acl}
    if(Test-Path -LiteralPath $base){
        if(-not ([IO.Path]::GetFullPath($base)).StartsWith($repo+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw '시험 정리 범위 오류'}
        foreach($entry in @(Get-Item -LiteralPath $base)+@(Get-ChildItem -LiteralPath $base -Recurse -Force)){if($entry.Attributes -band [IO.FileAttributes]::ReparsePoint){throw '시험 링크 보존'}}
        Remove-Item -LiteralPath $base -Recurse -Force
    }
}
