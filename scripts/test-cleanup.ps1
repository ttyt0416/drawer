$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'cleanup.ps1')
$project = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$fixture = Join-Path $project ('.cache-cleanup-test-' + [Guid]::NewGuid().ToString('N'))
[void](New-Item -ItemType Directory -Path $fixture)
$root = Join-Path $fixture 'cache'
$link = Join-Path $root 'debug/link'
function Write-Fixture([string]$Path,[string]$Value='fixture') {
    [void](New-Item -ItemType Directory -Force -Path ([IO.Path]::GetDirectoryName($Path)))
    [IO.File]::WriteAllText($Path,$Value)
}
function Expect-Rejection([scriptblock]$Action) {
    $rejected=$false
    try { & $Action | Out-Null } catch { $rejected=$true }
    if (-not $rejected) { throw '거절해야 할 캐시 입력이 허용되었습니다.' }
}
try {
    Write-Fixture (Join-Path $root 'release/drawer-app.exe') 'new executable'
    Write-Fixture (Join-Path $root 'release/drawer_app.pdb') 'new symbols'
    Write-Fixture (Join-Path $root 'release/drawer-app.d')
    foreach ($extension in 'exe','pdb','d') { Write-Fixture (Join-Path $root ('release/deps/drawer_app.'+$extension)) }
    Write-Fixture (Join-Path $root 'release/.fingerprint/drawer-app-1111111111111111/bin-drawer-app.json') '{}'
    Write-Fixture (Join-Path $root 'release/.fingerprint/drawer-app-1111111111111111/invoked.timestamp')
    Write-Fixture (Join-Path $root 'release/.fingerprint/drawer-app-0000000000000000/bin-drawer-app.json') '{}'
    Write-Fixture (Join-Path $root 'release/.fingerprint/drawer-app-0000000000000000/invoked.timestamp')
    (Get-Item -LiteralPath (Join-Path $root 'release/.fingerprint/drawer-app-0000000000000000/invoked.timestamp')).LastWriteTimeUtc=[DateTime]::UtcNow.AddDays(-1)
    foreach ($hash in '2222222222222222','3333333333333333','4444444444444444') {
        foreach ($ext in 'rlib','rmeta') { Write-Fixture (Join-Path $root "release/deps/libsame_name-$hash.$ext") }
        Write-Fixture (Join-Path $root "release/deps/same_name-$hash.d")
        Write-Fixture (Join-Path $root "release/.fingerprint/same-name-$hash/lib-same_name.json") '{}'
    }
    Write-Fixture (Join-Path $root 'release/deps/same_name-2222222222222222.long-type-stale.txt')
    Write-Fixture (Join-Path $root 'release/build/native-5555555555555555/build-script-build.exe')
    Write-Fixture (Join-Path $root 'release/build/native-6666666666666666/out/native.lib')
    Write-Fixture (Join-Path $root 'release/.fingerprint/native-5555555555555555/build-script-build-script-build.json') '{}'
    Write-Fixture (Join-Path $root 'release/.fingerprint/native-6666666666666666/run-build-script-build-script-build.json') '{}'
    Write-Fixture (Join-Path $root 'release/build/native-7777777777777777/out/old.lib')
    Write-Fixture (Join-Path $root 'debug/incremental/old/source.o')
    Write-Fixture (Join-Path $root 'release/incremental/old/source.o')
    Write-Fixture (Join-Path $root 'release/old-tool.exe')
    $outside=Join-Path $fixture 'outside/sentinel.txt'
    Write-Fixture $outside 'preserve outside cache'
    $graph=@(
        @{reason='compiler-artifact';target=@{name='drawer-app'};filenames=@((Join-Path $root 'release/drawer-app.exe'),(Join-Path $root 'release/drawer_app.pdb'))},
        @{reason='compiler-artifact';target=@{name='same_name'};filenames=@((Join-Path $root 'release/deps/libsame_name-2222222222222222.rlib'),(Join-Path $root 'release/deps/libsame_name-2222222222222222.rmeta'))},
        @{reason='compiler-artifact';target=@{name='same_name'};filenames=@((Join-Path $root 'release/deps/libsame_name-3333333333333333.rlib'),(Join-Path $root 'release/deps/libsame_name-3333333333333333.rmeta'))},
        @{reason='compiler-artifact';target=@{name='build-script-build'};filenames=@((Join-Path $root 'release/build/native-5555555555555555/build-script-build.exe'))},
        @{reason='build-script-executed';out_dir=(Join-Path $root 'release/build/native-6666666666666666/out')},
        @{reason='build-finished';success=$true}
    ) | ForEach-Object {$_ | ConvertTo-Json -Depth 10 -Compress}
    $before=(Get-FileHash -LiteralPath (Join-Path $root 'release/drawer-app.exe')).Hash
    Expect-Rejection { Get-CachePrunePlan $root @('{}') }
    $failed=@($graph[0..4])+(@{reason='build-finished';success=$false}|ConvertTo-Json -Compress)
    Expect-Rejection { Get-CachePrunePlan $root $failed }
    $escaped=@(@{reason='compiler-artifact';filenames=@($outside)}|ConvertTo-Json -Compress)+$graph
    Expect-Rejection { Get-CachePrunePlan $root $escaped }
    Expect-Rejection { Remove-CachePrunePlan ([pscustomobject]@{Root=$root;Paths=@($outside)}) }
    [void](New-Item -ItemType Junction -Path $link -Target (Join-Path $fixture 'outside'))
    Expect-Rejection { Get-CachePrunePlan $root $graph }
    if ((Get-Content -LiteralPath $outside -Raw) -ne 'preserve outside cache') { throw '링크 밖 파일이 변경되었습니다.' }
    Remove-Item -LiteralPath $link -Force
    $plan=Get-CachePrunePlan $root $graph
    Remove-CachePrunePlan $plan
    foreach ($relative in 'debug','release/incremental','release/deps/libsame_name-4444444444444444.rlib','release/.fingerprint/drawer-app-0000000000000000','release/build/native-7777777777777777','release/old-tool.exe','release/deps/same_name-2222222222222222.long-type-stale.txt') {
        if (Test-Path -LiteralPath (Join-Path $root $relative)) { throw "오래된 캐시가 남았습니다: $relative" }
    }
    foreach ($relative in 'release/drawer-app.exe','release/drawer_app.pdb','release/drawer-app.d','release/deps/drawer_app.exe','release/deps/drawer_app.pdb','release/deps/drawer_app.d','release/deps/libsame_name-2222222222222222.rlib','release/deps/libsame_name-3333333333333333.rlib','release/deps/same_name-3333333333333333.d','release/.fingerprint/drawer-app-1111111111111111','release/build/native-5555555555555555/build-script-build.exe','release/build/native-6666666666666666/out/native.lib','release/.fingerprint/native-6666666666666666') {
        if (-not (Test-Path -LiteralPath (Join-Path $root $relative))) { throw "필요한 캐시를 삭제했습니다: $relative" }
    }
    if ((Get-FileHash -LiteralPath (Join-Path $root 'release/drawer-app.exe')).Hash -ne $before -or (Get-Content -LiteralPath $outside -Raw) -ne 'preserve outside cache') { throw '보존 대상이 변경되었습니다.' }
    Write-Output '캐시 정리 검증 통과: 최신/복수 버전 의존성·빌드 스크립트 보존, 구형/증분 제거, 실패/경로 이탈/중첩 링크 거절.'
} finally {
    if (Test-Path -LiteralPath $link) {
        $entry=Get-Item -LiteralPath $link -Force
        if (-not ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw '예상한 검증 링크가 아닙니다.' }
        Remove-Item -LiteralPath $link -Force
    }
    $absolute=[IO.Path]::GetFullPath($fixture)
    if (-not $absolute.StartsWith($project+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase) -or [IO.Path]::GetFileName($absolute) -notlike '.cache-cleanup-test-*') { throw '검증 폴더 경로가 안전하지 않습니다.' }
    foreach ($entry in Get-ChildItem -LiteralPath $absolute -Recurse -Force) { if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '검증 폴더에 예상 밖 링크가 있습니다.' } }
    Remove-Item -LiteralPath $absolute -Recurse -Force
}
