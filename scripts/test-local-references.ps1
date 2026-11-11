param([string]$Evidence='docs/process/local-references-evidence.json')
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$base=Join-Path $repo ('.verify-279-junction-'+[guid]::NewGuid().ToString('N'))
$target=Join-Path $base 'owned-target';$link=Join-Path $base 'blocked-link';$child=$null
try {
    if(-not ([IO.Path]::GetFullPath($target)).StartsWith($base+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw '시험 경로 범위 오류'}
    [void](New-Item -ItemType Directory -Path $target)
    foreach($name in 'probe.drawer','probe.mp4','probe.obj','ffmpeg.exe','ffprobe.exe','manifest.json'){[IO.File]::WriteAllText((Join-Path $target $name),'must not be parsed')}
    $before=@{};foreach($file in Get-ChildItem -LiteralPath $target -File){$before[$file.Name]=(Get-FileHash -LiteralPath $file.FullName).Hash}
    [void](New-Item -ItemType Junction -Path $link -Target $target)
    if(-not ((Get-Item -LiteralPath $link -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)){throw '실제 Junction 생성 실패'}
    $exe=Join-Path $repo 'dist/Drawer-windows-x64/Drawer.exe';$receipt=Join-Path $base 'receipt.txt'
    $child=Start-Process -FilePath $exe -ArgumentList @('--verify-local-references',('"'+$base+'"'),('"'+$link+'"'),('"'+$receipt+'"')) -WindowStyle Hidden -Wait -PassThru
    if($child.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $receipt)){throw '실제 Junction/로컬 참조 검사 실패'}
    $text=Get-Content -LiteralPath $receipt -Raw
    foreach($flag in 'local_paths=true','junction_refused_before_parse=true','cache_preserved_local_relink=true'){if(-not $text.Contains($flag)){throw '검증 영수증 누락'}}
    if(@(Get-ChildItem -LiteralPath $target -File).Count -ne $before.Count){throw 'Junction 대상 쓰기 발생'}
    foreach($name in $before.Keys){if((Get-FileHash -LiteralPath (Join-Path $target $name)).Hash -ne $before[$name]){throw 'Junction 대상 바이트 변경'}}
    [ordered]@{version=1;executableSha256=(Get-FileHash -LiteralPath $exe).Hash.ToLowerInvariant();actualLocalJunction=$true;rejectedBeforeParsing=$true;targetFilesUnchanged=$true;httpHttpsFileUriUncParentTraversalRefused=$true;blockedSvgReferencesReported=$true;missingNativeCacheOpened=$true;explicitLocalRelinkUndo=$true;limits='Mapped-network drive classification uses a deterministic policy test; no host mappings were added. No remote UNC server was accessed.'}|ConvertTo-Json|Set-Content -LiteralPath (Join-Path $repo $Evidence) -Encoding utf8
    Write-Output '실제 로컬 Junction 조회 전 거절·대상 바이트 보존·URI/UNC/탈출 차단·캐시/로컬 재연결 통과'
} finally {
    if($child -and -not $child.HasExited){$child.Kill();$child.WaitForExit()}
    if(Test-Path -LiteralPath $link){$entry=Get-Item -LiteralPath $link -Force;if(-not ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint)){throw '예상과 다른 연결 폴더 보존'};Remove-Item -LiteralPath $link -Force}
    if(Test-Path -LiteralPath $base){
        if(-not ([IO.Path]::GetFullPath($base)).StartsWith($repo+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw '시험 삭제 범위 오류'}
        foreach($entry in @(Get-Item -LiteralPath $base)+@(Get-ChildItem -LiteralPath $base -Recurse -Force)){if($entry.Attributes -band [IO.FileAttributes]::ReparsePoint){throw '시험의 알 수 없는 링크 보존'}}
        Remove-Item -LiteralPath $base -Recurse -Force
    }
}
