param([string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs/Drawer'))
$ErrorActionPreference = 'Stop'
$installationRoot = [IO.Path]::GetFullPath($InstallDir)
if ($installationRoot -eq [IO.Path]::GetPathRoot($installationRoot)) { throw '드라이브 루트에는 설치할 수 없습니다.' }
$files = @('Drawer.exe', 'THIRD-PARTY-NOTICES.txt', 'COMPONENTS.json', 'Cargo.lock', 'OWN-ASSETS.md', 'START-HERE.md', 'MOVIE-IMPORT.md', 'Install.ps1', 'Uninstall.ps1')
foreach ($name in $files) { if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot $name) -PathType Leaf)) { throw "설치 파일 누락: $name" } }
if (Test-Path -LiteralPath $installationRoot) {
    if ((Get-Item -LiteralPath $installationRoot).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '설치 폴더는 링크일 수 없습니다.' }
    if ((Get-ChildItem -LiteralPath $installationRoot -Force) -and -not (Test-Path -LiteralPath (Join-Path $installationRoot '.drawer-install.json'))) { throw '다른 파일이 있는 폴더에는 설치하지 않습니다.' }
    if (Test-Path -LiteralPath (Join-Path $installationRoot '.drawer-install.json')) {
        $previous = Get-Content -LiteralPath (Join-Path $installationRoot '.drawer-install.json') -Raw | ConvertFrom-Json
        if ($previous.product -ne 'Drawer' -or $previous.installRoot -ne $installationRoot) { throw '이전 설치 기록과 경로가 다릅니다.' }
    }
} else { New-Item -ItemType Directory -Path $installationRoot | Out-Null }
foreach ($name in $files) {
    $destination = Join-Path $installationRoot $name
    if ((Test-Path -LiteralPath $destination) -and ((Get-Item -LiteralPath $destination).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw '설치 대상 파일은 링크일 수 없습니다.' }
    if ([IO.Path]::GetFullPath((Join-Path $PSScriptRoot $name)) -ne $destination) { Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $destination -Force }
}
@{ product = 'Drawer'; installRoot = $installationRoot; files = $files } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $installationRoot '.drawer-install.json') -Encoding utf8
Write-Output "Drawer 설치 완료: $installationRoot"
