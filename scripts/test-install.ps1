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
$oldDiagnostics = $env:DRAWER_SMOKE_DIAGNOSTICS
try {
    & (Join-Path $packageRoot 'Install.ps1') -InstallDir $installRoot
    $env:DRAWER_DATA_DIR = $dataRoot
    $env:DRAWER_SMOKE_DIAGNOSTICS = $testRoot
    $env:PATH = Join-Path $env:SystemRoot 'System32'
    $receipt = Join-Path $testRoot 'receipt.txt'
    $errors = Join-Path $testRoot 'errors.txt'
    $process = Start-Process -FilePath (Join-Path $installRoot 'Drawer.exe') -ArgumentList @('--smoke-test', ('"' + $receipt + '"')) -WorkingDirectory $testRoot -WindowStyle Hidden -RedirectStandardError $errors -PassThru
    if (-not $process.WaitForExit(30000)) { $process.Kill(); throw '설치 앱 시작 시간 초과' }
    if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $receipt)) { Get-Content -LiteralPath $errors; if (Test-Path -LiteralPath (Join-Path $testRoot 'smoke-failure.txt')) { Get-Content -LiteralPath (Join-Path $testRoot 'smoke-failure.txt') }; throw '설치 앱 실행 실패' }
    $preferences = Join-Path $dataRoot 'settings/preferences.txt'
    if (-not (Test-Path -LiteralPath $preferences)) { throw '사용자 설정 저장 실패' }
    $before = (Get-FileHash -LiteralPath $preferences).Hash
    $referenceSettings = Join-Path $dataRoot 'settings/references.json'
    $referenceCopy = Join-Path $dataRoot 'materials/references/reference-1.png'
    $aidSettings = Join-Path $dataRoot 'settings/aids.json'
    $viewSettings = Join-Path $dataRoot 'settings/view.txt'
    $recentSettings = Join-Path $dataRoot 'settings/recent.json'
    $displaySettings = Join-Path $dataRoot 'settings/display.json'
    $presetSettings = Join-Path $dataRoot 'settings/canvas-presets.json'
    $interpolationSettings = Join-Path $dataRoot 'settings/interpolation.json'
    $layoutSettings = Join-Path $dataRoot 'settings/views.json'
    $navigatorSettings = Join-Path $dataRoot 'settings/navigator.json'
    $panelSettings = Join-Path $dataRoot 'settings/panels.json'
    $workspaceSettings = Join-Path $dataRoot 'settings/workspaces.json'
    $workspaceBackup = Join-Path $dataRoot 'settings/workspace-before-apply.json'
    $interfaceSettings = Join-Path $dataRoot 'settings/interface.json'
    $referenceHashes = @{}
    foreach ($referenceFile in $referenceSettings,$referenceCopy,$aidSettings,$viewSettings,$recentSettings,$displaySettings,$presetSettings,$interpolationSettings,$layoutSettings,$navigatorSettings,$panelSettings,$interfaceSettings,$workspaceSettings,$workspaceBackup) {
        if (-not (Test-Path -LiteralPath $referenceFile)) { throw '참고 설정/복사본 저장 실패' }
        $referenceHashes[$referenceFile] = (Get-FileHash -LiteralPath $referenceFile).Hash
    }
    New-Item -ItemType Directory -Path (Join-Path $dataRoot 'documents') | Out-Null
    Set-Content -LiteralPath (Join-Path $dataRoot 'documents/user-picture.txt') -Value 'keep document'
    Set-Content -LiteralPath (Join-Path $dataRoot 'materials/user-brush.txt') -Value 'keep material'
    Set-Content -LiteralPath (Join-Path $installRoot 'user-note.txt') -Value 'keep extra file'
    & (Join-Path $packageRoot 'Install.ps1') -InstallDir $installRoot
    if ((Get-FileHash -LiteralPath $preferences).Hash -ne $before) { throw '재설치가 설정을 변경했습니다.' }
    foreach ($referenceFile in $referenceHashes.Keys) { if ((Get-FileHash -LiteralPath $referenceFile).Hash -ne $referenceHashes[$referenceFile]) { throw '재설치가 참고 설정/복사본을 변경했습니다.' } }
    & (Join-Path $packageRoot 'Uninstall.ps1') -InstallDir $installRoot
    foreach ($file in (Join-Path $dataRoot 'documents/user-picture.txt'), (Join-Path $dataRoot 'materials/user-brush.txt'), $preferences, (Join-Path $installRoot 'user-note.txt'), $referenceSettings, $referenceCopy, $aidSettings, $viewSettings, $recentSettings, $displaySettings, $presetSettings, $interpolationSettings, $layoutSettings, $navigatorSettings, $panelSettings, $interfaceSettings, $workspaceSettings, $workspaceBackup) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "사용자 파일 보존 실패: $file" }
    }
    if (Test-Path -LiteralPath (Join-Path $installRoot 'Drawer.exe')) { throw '프로그램 제거 실패' }
    $smokeResult = Get-Content -LiteralPath $receipt -Raw
    if ($smokeResult -notmatch 'canvases=4' -or $smokeResult -notmatch 'views=4' -or $smokeResult -notmatch 'close_cancel=true') { throw '문서 전환/닫기 취소 시작 시험 실패' }
    if ($smokeResult -notmatch 'sizing_dpi=true' -or $smokeResult -notmatch 'sizing_undo=true') { throw 'DPI 변경/취소 시작 시험 실패' }
    if ($smokeResult -notmatch 'reference_copy=true' -or $smokeResult -notmatch 'reference_document=true') { throw '참고 복사본/독립 문서 시작 시험 실패' }
    if ($smokeResult -notmatch 'grid_guide_undo=true' -or $smokeResult -notmatch 'print_size_view=true') { throw '격자/가이드/인쇄 크기 시작 시험 실패' }
    if ($smokeResult -notmatch 'clipboard_document=true' -or $smokeResult -notmatch 'save_as_export=true' -or $smokeResult -notmatch 'recent_restart=true' -or $smokeResult -notmatch 'image_decode=true') { throw '클립보드/다른 이름 저장/최근 파일/이미지 가져오기 시작 시험 실패' }
    if ($smokeResult -notmatch 'native_display_view=true' -or $smokeResult -notmatch 'monitor_inventory=true') { throw '별도 보기 창/모니터 목록 시작 시험 실패' }
    if ($smokeResult -notmatch 'canvas_presets=true' -or $smokeResult -notmatch 'signed_edges_undo=true') { throw '크기 프리셋/각 변 증감 시작 시험 실패' }
    if ($smokeResult -notmatch 'paste_layer_undo=true' -or $smokeResult -notmatch 'paste_native_reopen=true') { throw '붙여넣기 레이어/실행 취소/재열기 시작 시험 실패' }
    if ($smokeResult -notmatch 'interpolation_view=true' -or $smokeResult -notmatch 'interpolation_settings=true') { throw '보기 보간/설정 저장 시작 시험 실패' }
    if ($smokeResult -notmatch 'view_layout=true') { throw '보기 순서/복원 시작 시험 실패' }
    if ($smokeResult -notmatch 'navigator_settings=true') { throw '내비게이터 설정 저장 시작 시험 실패' }
    if ($smokeResult -notmatch 'revert_saved=true' -or $smokeResult -notmatch 'movie_decoder_missing=true' -or $smokeResult -notmatch 'animation_native=true') { throw '되돌리기/로컬 디코더 누락/애니메이션 재열기 시작 시험 실패' }
    if ($smokeResult -notmatch 'panel_settings=true') { throw '패널 설정 저장/복원 시작 시험 실패' }
    if ($smokeResult -notmatch 'interface_settings=true') { throw '화면 배율 설정 저장/복원 시작 시험 실패' }
    if ($smokeResult -notmatch 'workspace_settings=true' -or $smokeResult -notmatch 'native_floating_panels=true' -or $smokeResult -notmatch 'popup_test_painting=true') { throw '작업공간/분리 패널/시험 칠하기 시작 시험 실패' }
    if ($smokeResult -notmatch 'command_settings=true' -or $smokeResult -notmatch 'tool_cycle=true' -or $smokeResult -notmatch 'radial_settings=true' -or $smokeResult -notmatch 'device_action_profile=true') { throw '단축키/순환/원형 메뉴/장치 배정 저장 시작 시험 실패' }
    if ($smokeResult -notmatch 'temporary_tool=true' -or $smokeResult -notmatch 'drag_threshold=true') { throw '임시 도구/드래그 임계값 설정 시작 시험 실패' }
    if ($smokeResult -notmatch 'local_help=true' -or $smokeResult -notmatch 'local_help_licenses=true' -or $smokeResult -notmatch 'local_korean_font=true') { throw '로컬 도움말/고지/한글 글꼴 시작 시험 실패' }
    foreach ($localHelpFile in 'HELP.md','IMAGE-IMPORT.md','MOVIE-IMPORT.md') { if (-not (Test-Path -LiteralPath (Join-Path $packageRoot $localHelpFile))) { throw '로컬 도움말 배포 누락' } }
    if ($smokeResult -notmatch 'panel_auto_hide=true' -or $smokeResult -notmatch 'local_power_release=true') { throw '패널 자동 숨김/절전 요청 해제 시작 시험 실패' }
    if ($smokeResult -notmatch 'cursor_modes=true') { throw '원시/보정/예측 커서 설정 재열기 시작 시험 실패' }
    if ($smokeResult -notmatch 'information_panel=true' -or $smokeResult -notmatch 'local_color_profile=true' -or $smokeResult -notmatch 'theme_settings=true') { throw '좌표/색 정보/테마 설정 시작 시험 실패' }
    if ($smokeResult -notmatch 'cursor_assets=true') { throw '브러시 커서 도구 소재 시작 시험 실패' }
    if ($smokeResult -notmatch 'modifier_tools=true') { throw '수정키 임시 도구 설정 시험 실패' }
    if ($smokeResult -notmatch 'screen_modes=true') { throw '간단/전체 화면 전환 시험 실패' }
    if ($smokeResult -notmatch 'context_input=true') { throw '상황 도움말/입력 제어기 등록 시험 실패' }
    if ($smokeResult -notmatch 'layout_windows=true') { throw '패널 잠금/캔버스 창 정렬 시험 실패' }
    if ($smokeResult -notmatch 'tool_property_settings=true') { throw '도구별 속성 설정 시작 시험 실패' }
    if ($smokeResult -notmatch 'ime_backend=(toggle-and-restore|provider-declined-state-preserved)') { throw '자체 창 IME 상태 복원 시험 실패' }
    Get-Content -LiteralPath $receipt
    Write-Output '설치·실행·설정 저장·재설치·제거 시험 통과; 개발도구 PATH 없이 시작; 사용자 파일 18개 보존.'
} finally {
    $env:DRAWER_DATA_DIR = $oldData
    $env:PATH = $oldPath
    $env:DRAWER_SMOKE_DIAGNOSTICS = $oldDiagnostics
    $resolvedTest = [IO.Path]::GetFullPath($testRoot)
    if ($resolvedTest -ne (Join-Path $projectRoot '.installation-test')) { throw '시험 정리 경로 불일치' }
    if ((Get-Item -LiteralPath $resolvedTest).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '시험 정리 대상은 링크일 수 없습니다.' }
    Remove-Item -LiteralPath $resolvedTest -Recurse -Force
}
