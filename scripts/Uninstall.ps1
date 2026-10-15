param([string]$InstallDir = $PSScriptRoot)
$ErrorActionPreference = 'Stop'
$installationRoot = [IO.Path]::GetFullPath($InstallDir)
if ($installationRoot -eq [IO.Path]::GetPathRoot($installationRoot)) { throw '제거 대상 경로가 잘못되었습니다.' }
if ((Get-Item -LiteralPath $installationRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '설치 폴더는 링크일 수 없습니다.' }
$markerPath = Join-Path $installationRoot '.drawer-install.json'
$marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
if ($marker.product -ne 'Drawer' -or $marker.installRoot -ne $installationRoot) { throw 'Drawer 설치 기록과 경로가 다릅니다.' }
$allowed = @('Drawer.exe', 'THIRD-PARTY-NOTICES.txt', 'COMPONENTS.json', 'Cargo.lock', 'OWN-ASSETS.md', 'START-HERE.md', 'MOVIE-IMPORT.md', 'IMAGE-IMPORT.md', 'HELP.md', 'Install.ps1', 'Uninstall.ps1')
foreach ($name in $marker.files) { if ($name -notin $allowed) { throw '설치 기록에 허용하지 않는 제거 경로가 있습니다.' } }
foreach ($name in $marker.files) {
    $filePath = [IO.Path]::GetFullPath((Join-Path $installationRoot $name))
    if (-not $filePath.StartsWith($installationRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw '제거 경로가 설치 폴더를 벗어났습니다.' }
    if (Test-Path -LiteralPath $filePath -PathType Leaf) { Remove-Item -LiteralPath $filePath -Force }
}
Remove-Item -LiteralPath $markerPath -Force
if (-not (Get-ChildItem -LiteralPath $installationRoot -Force)) { Remove-Item -LiteralPath $installationRoot -Force }
Write-Output 'Drawer 프로그램 제거 완료. 사용자 문서·소재·설정은 보존했습니다.'
