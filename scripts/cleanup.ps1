param([ValidatePattern('^(item-[0-9]{3}|latest)$')][string]$KeepBuild = 'latest')
$ErrorActionPreference = 'Stop'

function Assert-CachePath {
    param([string]$Root, [string]$Path)
    $rootPath = [IO.Path]::GetFullPath($Root).TrimEnd([IO.Path]::DirectorySeparatorChar)
    $absolute = [IO.Path]::GetFullPath($Path)
    if (-not $absolute.StartsWith($rootPath + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "캐시 범위를 벗어난 경로: $absolute"
    }
    if (Test-Path -LiteralPath $absolute) {
        $entry = Get-Item -LiteralPath $absolute -Force
        if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "캐시 링크는 정리하지 않습니다: $absolute" }
    }
    return $absolute
}

function Get-CachePrunePlan {
    param([string]$BuildRoot, [string[]]$ArtifactLines)
    $root = [IO.Path]::GetFullPath($BuildRoot).TrimEnd([IO.Path]::DirectorySeparatorChar)
    if ((Get-Item -LiteralPath $root -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '캐시 루트는 링크일 수 없습니다.' }
    $release = Join-Path $root 'release'
    $keepFiles = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $hashes = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $buildDirs = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $bins = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $finished = $false
    $artifacts = 0
    foreach ($line in $ArtifactLines) {
        if ([string]::IsNullOrWhiteSpace($line)) { continue }
        $event = $line | ConvertFrom-Json -ErrorAction Stop
        if ($event.reason -eq 'build-finished') { $finished = $event.success -eq $true }
        if ($event.reason -eq 'build-script-executed') {
            $output = Assert-CachePath $root $event.out_dir
            $relative = [IO.Path]::GetRelativePath($release, $output).Split([IO.Path]::DirectorySeparatorChar)
            if ($relative.Count -lt 3 -or $relative[0] -ne 'build' -or $relative[1] -notmatch '-([a-f0-9]{16})$') { throw '예상하지 못한 빌드 스크립트 경로' }
            [void]$buildDirs.Add($relative[1])
            [void]$hashes.Add($Matches[1])
        }
        if ($event.reason -ne 'compiler-artifact') { continue }
        $artifacts++
        foreach ($name in @($event.filenames)) {
            $file = Assert-CachePath $root $name
            if (-not $file.StartsWith($release + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw '릴리스 밖의 산출물이 포함되었습니다.' }
            if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "최신 산출물이 없습니다: $file" }
            [void]$keepFiles.Add($file)
            $relative = [IO.Path]::GetRelativePath($release, $file).Split([IO.Path]::DirectorySeparatorChar)
            if ($relative[0] -eq 'build') {
                if ($relative[1] -notmatch '-([a-f0-9]{16})$') { throw '빌드 디렉터리 해시가 없습니다.' }
                [void]$buildDirs.Add($relative[1]); [void]$hashes.Add($Matches[1])
            } elseif ($relative[0] -eq 'deps') {
                if ($relative[-1] -notmatch '-([a-f0-9]{16})\.') { throw '산출물 해시가 없습니다.' }
                [void]$hashes.Add($Matches[1])
            } else {
                [void]$bins.Add($event.target.name)
                [void]$keepFiles.Add((Join-Path $release ($event.target.name + '.d')))
                # Cargo lists the public executable/PDB, but freshness also requires the
                # crate-name copies and dep-info in deps (hyphens become underscores).
                $crateName=$event.target.name.Replace('-','_')
                [void]$keepFiles.Add((Join-Path $release ('deps/' + $crateName + '.d')))
                [void]$keepFiles.Add((Join-Path $release ('deps/' + $crateName + '.exe')))
                [void]$keepFiles.Add((Join-Path $release ('deps/' + $crateName + '.pdb')))

            }
        }
    }
    $main = Join-Path $release 'drawer-app.exe'
    if (-not $finished -or $artifacts -eq 0 -or -not $keepFiles.Contains($main)) { throw '완료된 최신 앱 빌드 목록 없이는 캐시를 삭제하지 않습니다.' }
    foreach ($bin in $bins) {
        $marker = 'bin-' + $bin + '.json'
        $matchesForBin = @(Get-ChildItem -LiteralPath (Join-Path $release '.fingerprint') -Directory -Force | Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName $marker) } | Sort-Object { (Get-Item -LiteralPath (Join-Path $_.FullName 'invoked.timestamp')).LastWriteTimeUtc } -Descending)
        if ($matchesForBin.Count -eq 0 -or $matchesForBin[0].Name -notmatch '-([a-f0-9]{16})$') { throw "최신 실행 파일의 빌드 기록이 없습니다: $bin" }
        [void]$hashes.Add($Matches[1])
    }
    # Inspect the entire tree before removing anything, including links nested in obsolete folders.
    $entries = @(Get-ChildItem -LiteralPath $root -Recurse -Force)
    foreach ($entry in $entries) { [void](Assert-CachePath $root $entry.FullName) }
    $delete = [Collections.Generic.List[string]]::new()
    $directories = @($entries | Where-Object PSIsContainer | Sort-Object { $_.FullName.Length })
    foreach ($directory in $directories) {
        $relative = [IO.Path]::GetRelativePath($root, $directory.FullName).Split([IO.Path]::DirectorySeparatorChar)
        $retain = $relative[0] -eq 'release'
        if ($retain -and $relative.Count -ge 2) {
            switch ($relative[1]) {
                'deps' { $retain = $relative.Count -eq 2 }
                'build' { $retain = $relative.Count -eq 2 -or $buildDirs.Contains($relative[2]) }
                '.fingerprint' { $retain = $relative.Count -eq 2 -or ($relative[2] -match '-([a-f0-9]{16})$' -and $hashes.Contains($Matches[1])) }
                default { $retain = $false }
            }
        }
        if (-not $retain -and -not ($delete | Where-Object { $directory.FullName.StartsWith($_ + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) })) { $delete.Add($directory.FullName) }
    }
    foreach ($file in $entries | Where-Object { -not $_.PSIsContainer }) {
        if ($delete | Where-Object { $file.FullName.StartsWith($_ + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) }) { continue }
        $relative = [IO.Path]::GetRelativePath($root, $file.FullName).Split([IO.Path]::DirectorySeparatorChar)
        $retain = $keepFiles.Contains($file.FullName)
        if ($relative.Count -eq 1) { $retain = $file.Name -in '.rustc_info.json','CACHEDIR.TAG','active-release-artifacts.jsonl' }
        elseif ($relative[0] -eq 'release') {
            if ($relative.Count -eq 2 -and $file.Name -in '.cargo-lock','.cargo-build-lock','.cargo-artifact-lock') { $retain = $true }
            elseif ($relative[1] -eq 'deps' -and $file.Name -match '-([a-f0-9]{16})\.') { $retain = $hashes.Contains($Matches[1]) }
            elseif ($relative[1] -eq 'build' -and $relative.Count -ge 4) { $retain = $buildDirs.Contains($relative[2]) }
            elseif ($relative[1] -eq '.fingerprint' -and $relative.Count -ge 4) { $retain = $relative[2] -match '-([a-f0-9]{16})$' -and $hashes.Contains($Matches[1]) }
        }
        if ($file.Name -like "*.long-type-*.txt" -or $file.Extension -in ".tmp", ".bak") { $retain=$false }
        if (-not $retain) { $delete.Add($file.FullName) }
    }
    [pscustomobject]@{ Root=$root; Artifacts=$artifacts; Paths=@($delete); FileBytes=($entries | Where-Object { -not $_.PSIsContainer } | Measure-Object Length -Sum).Sum }
}

function Remove-CachePrunePlan {
    param($Plan)
    foreach ($name in $Plan.Paths) {
        $absolute = Assert-CachePath $Plan.Root $name
        if (Test-Path -LiteralPath $absolute -PathType Container) {
            foreach ($child in Get-ChildItem -LiteralPath $absolute -Recurse -Force) { [void](Assert-CachePath $Plan.Root $child.FullName) }
            Remove-Item -LiteralPath $absolute -Recurse -Force
        } else { Remove-Item -LiteralPath $absolute -Force }
    }
}

# Dot sourcing exposes only the bounded planning/removal functions to fixture tests.
if ($MyInvocation.InvocationName -eq '.') { return }
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cacheRoot = Join-Path $projectRoot 'target'
if ((Get-Item -LiteralPath $cacheRoot -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '빌드 캐시 루트는 링크일 수 없습니다.' }
$keepPath = Assert-CachePath $cacheRoot (Join-Path $cacheRoot $KeepBuild)
if (-not (Test-Path -LiteralPath (Join-Path $keepPath 'release/drawer-app.exe') -PathType Leaf)) { throw '검증된 최신 릴리스 빌드가 없습니다. 먼저 package.ps1을 실행하세요.' }
Push-Location $projectRoot
try {
    $beforeHash = (Get-FileHash -LiteralPath (Join-Path $keepPath 'release/drawer-app.exe')).Hash
    $graph = @(& cargo rustc -p drawer-app --release --locked --offline --target-dir $keepPath --message-format=json -- -C target-feature=+crt-static)
    if ($LASTEXITCODE -ne 0) { throw '최신 릴리스 산출물 확인 실패; 정리를 중단합니다.' }
    if (@(Get-Process cargo,rustc -ErrorAction SilentlyContinue).Count -ne 0) { throw '다른 빌드가 실행 중이므로 정리를 중단합니다.' }
    $plan = Get-CachePrunePlan $keepPath $graph
    if ((Get-FileHash -LiteralPath (Join-Path $keepPath 'release/drawer-app.exe')).Hash -ne $beforeHash) { throw '릴리스가 갱신되었습니다. 새 빌드를 검증한 뒤 정리를 다시 실행하세요.' }
    Remove-CachePrunePlan $plan
    foreach ($entry in Get-ChildItem -LiteralPath $cacheRoot -Directory -Force) {
        if ($entry.Name -eq $KeepBuild) { continue }
        $absolute = Assert-CachePath $cacheRoot $entry.FullName
        foreach ($child in Get-ChildItem -LiteralPath $absolute -Recurse -Force) { [void](Assert-CachePath $cacheRoot $child.FullName) }
        Remove-Item -LiteralPath $absolute -Recurse -Force
    }
    $verification = @(& cargo rustc -p drawer-app --release --locked --offline --target-dir $keepPath --message-format=json -- -C target-feature=+crt-static)
    if ($LASTEXITCODE -ne 0) { throw '정리 후 릴리스 캐시 확인 실패' }
    $verifiedEvents = @($verification | ForEach-Object { $_ | ConvertFrom-Json -ErrorAction Stop })
    $verifiedArtifacts = @($verifiedEvents | Where-Object reason -eq 'compiler-artifact')
    if ($verifiedArtifacts.Count -ne $plan.Artifacts -or @($verifiedArtifacts | Where-Object { -not $_.fresh }).Count -ne 0 -or -not ($verifiedEvents | Where-Object { $_.reason -eq 'build-finished' -and $_.success })) {
        throw '정리 후 캐시가 재생성되었습니다. 보존 목록 확인이 필요합니다.'
    }
    $verification | Set-Content -LiteralPath (Join-Path $keepPath 'active-release-artifacts.jsonl') -Encoding utf8
    # Cargo creates these empty directories even for a fully fresh release build.
    foreach ($name in 'incremental','examples') {
        $empty = Assert-CachePath $keepPath (Join-Path $keepPath ('release/' + $name))
        if ((Test-Path -LiteralPath $empty -PathType Container) -and @(Get-ChildItem -LiteralPath $empty -Force).Count -eq 0) { Remove-Item -LiteralPath $empty -Force }
    }

    if ((Get-FileHash -LiteralPath (Join-Path $keepPath 'release/drawer-app.exe')).Hash -ne $beforeHash) { throw '최신 실행 파일 보존 검증 실패' }
    $afterBytes = (Get-ChildItem -LiteralPath $keepPath -File -Recurse -Force | Measure-Object Length -Sum).Sum
    Write-Output ('캐시 정리: {0:N3}GiB → {1:N3}GiB; 최신 릴리스 항목 {2}개 유지. debug/incremental 및 오래된 산출물 제거. 재컴파일0개로 재사용 확인.' -f ($plan.FileBytes/1GB),($afterBytes/1GB),$plan.Artifacts)
    if (Test-Path -LiteralPath (Join-Path $projectRoot '.handoff-staging')) { throw '임시 전달 폴더가 남아 있습니다.' }
} finally { Pop-Location }
