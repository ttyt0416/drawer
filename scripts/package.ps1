$ErrorActionPreference = 'Stop'
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location $projectRoot
try {
    python -X utf8 scripts/license-inventory.py --check
    if ($LASTEXITCODE -ne 0) { throw '라이선스 목록 검증 실패' }
    cargo rustc -p drawer-app --release --locked --offline --target-dir target/latest -- -C target-feature=+crt-static
    if ($LASTEXITCODE -ne 0) { throw '앱 빌드 실패' }
    $packageRoot = Join-Path $projectRoot 'dist/Drawer-windows-x64'
    New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot 'target/latest/release/drawer-app.exe') -Destination (Join-Path $packageRoot 'Drawer.exe')
    foreach ($entry in @{
        'docs/licenses/third-party-notices.txt' = 'THIRD-PARTY-NOTICES.txt'
        'docs/licenses/inventory.json' = 'COMPONENTS.json'
        'Cargo.lock' = 'Cargo.lock'
        'docs/process/own-assets.md' = 'OWN-ASSETS.md'
        'docs/START-HERE.md' = 'START-HERE.md'
        'scripts/Install.ps1' = 'Install.ps1'
        'scripts/Uninstall.ps1' = 'Uninstall.ps1'
    }.GetEnumerator()) {
        Copy-Item -LiteralPath (Join-Path $projectRoot $entry.Key) -Destination (Join-Path $packageRoot $entry.Value)
    }
    foreach ($required in 'Drawer.exe', 'THIRD-PARTY-NOTICES.txt', 'COMPONENTS.json', 'Cargo.lock', 'OWN-ASSETS.md') {
        if (-not (Test-Path -LiteralPath (Join-Path $packageRoot $required) -PathType Leaf)) { throw "배포 파일 누락: $required" }
    }
    Write-Output "배포 묶음 준비: $packageRoot"
} finally { Pop-Location }
