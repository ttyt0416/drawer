$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$testRoot = Join-Path $projectRoot '.installation-test'
if (Test-Path -LiteralPath $testRoot) { throw '이전 설치 시험 폴더를 먼저 확인해야 합니다.' }
$installRoot = Join-Path $testRoot 'program'
$dataRoot = Join-Path $testRoot 'userdata'
$packageRoot = Join-Path $projectRoot 'dist/Drawer-windows-x64'
New-Item -ItemType Directory -Path $testRoot | Out-Null
$oldData = $env:DRAWER_DATA_DIR
$oldPath = $env:PATH
try {
    & (Join-Path $packageRoot 'Install.ps1') -InstallDir $installRoot
    $env:DRAWER_DATA_DIR = $dataRoot
    $env:PATH = Join-Path $env:SystemRoot 'System32'
    $receipt = Join-Path $testRoot 'receipt.txt'
    $process = Start-Process -FilePath (Join-Path $installRoot 'Drawer.exe') -ArgumentList @('--smoke-test', ('"' + $receipt + '"')) -WorkingDirectory $testRoot -WindowStyle Hidden -PassThru
    if (-not $process.WaitForExit(30000)) { $process.Kill(); throw '설치 앱 시작 시간 초과' }
    if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $receipt)) { throw '설치 앱 실행 실패' }
    $preferences = Join-Path $dataRoot 'settings/preferences.txt'
    if (-not (Test-Path -LiteralPath $preferences)) { throw '사용자 설정 저장 실패' }
    $before = (Get-FileHash -LiteralPath $preferences).Hash
    New-Item -ItemType Directory -Path (Join-Path $dataRoot 'documents') | Out-Null
    Set-Content -LiteralPath (Join-Path $dataRoot 'documents/user-picture.txt') -Value 'keep document'
    Set-Content -LiteralPath (Join-Path $dataRoot 'materials/user-brush.txt') -Value 'keep material'
    Set-Content -LiteralPath (Join-Path $installRoot 'user-note.txt') -Value 'keep extra file'
    & (Join-Path $packageRoot 'Install.ps1') -InstallDir $installRoot
    if ((Get-FileHash -LiteralPath $preferences).Hash -ne $before) { throw '재설치가 설정을 변경했습니다.' }
    & (Join-Path $packageRoot 'Uninstall.ps1') -InstallDir $installRoot
    foreach ($file in (Join-Path $dataRoot 'documents/user-picture.txt'), (Join-Path $dataRoot 'materials/user-brush.txt'), $preferences, (Join-Path $installRoot 'user-note.txt')) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "사용자 파일 보존 실패: $file" }
    }
    if (Test-Path -LiteralPath (Join-Path $installRoot 'Drawer.exe')) { throw '프로그램 제거 실패' }
    Get-Content -LiteralPath $receipt
    Write-Output '설치·실행·설정 저장·재설치·제거 시험 통과; 개발도구 PATH 없이 시작; 사용자 파일 4개 보존.'
} finally {
    $env:DRAWER_DATA_DIR = $oldData
    $env:PATH = $oldPath
    $resolvedTest = [IO.Path]::GetFullPath($testRoot)
    if ($resolvedTest -ne (Join-Path $projectRoot '.installation-test')) { throw '시험 정리 경로 불일치' }
    if ((Get-Item -LiteralPath $resolvedTest).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '시험 정리 대상은 링크일 수 없습니다.' }
    Remove-Item -LiteralPath $resolvedTest -Recurse -Force
}
