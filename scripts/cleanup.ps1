param([ValidatePattern('^(item-[0-9]{3}|latest)$')][string]$KeepBuild = 'latest')
$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$cacheRoot = Join-Path $projectRoot 'target'
if ((Get-Item -LiteralPath $cacheRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) {
    throw '빌드 캐시 폴더는 링크일 수 없습니다.'
}
$keepPath = Join-Path $cacheRoot $KeepBuild
if (-not (Test-Path -LiteralPath $keepPath -PathType Container)) { throw '유지할 최신 빌드 캐시가 없습니다.' }
$removedBuilds = 0
foreach ($entry in Get-ChildItem -LiteralPath $cacheRoot -Directory) {
    if ($entry.Name -eq $KeepBuild) { continue }
    $absolutePath = [IO.Path]::GetFullPath($entry.FullName)
    if (-not $absolutePath.StartsWith($cacheRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw '캐시 경로가 프로젝트 범위를 벗어났습니다.'
    }
    if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '캐시 링크는 자동 삭제하지 않습니다.' }
    Remove-Item -LiteralPath $absolutePath -Recurse -Force
    $removedBuilds++
}
$temporaryFiles = @(Get-ChildItem -LiteralPath $projectRoot -Recurse -File | Where-Object {
    $_.Extension -in '.tmp', '.bak' -and -not $_.FullName.StartsWith((Join-Path $projectRoot '.git') + '\')
})
foreach ($entry in $temporaryFiles) { Remove-Item -LiteralPath $entry.FullName -Force }
if (Test-Path -LiteralPath (Join-Path $projectRoot '.handoff-staging')) { throw '임시 전달 폴더가 남아 있습니다.' }
Write-Output "정리 완료: 이전 빌드 캐시 $removedBuilds 개, 임시 파일 $($temporaryFiles.Count) 개 삭제; $KeepBuild 유지."
