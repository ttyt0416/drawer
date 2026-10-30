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
$process = $null
try {
    & (Join-Path $packageRoot 'Install.ps1') -InstallDir $installRoot
    $env:DRAWER_DATA_DIR = $dataRoot
    $env:DRAWER_SMOKE_DIAGNOSTICS = $testRoot
    $env:PATH = Join-Path $env:SystemRoot 'System32'
    $receipt = Join-Path $testRoot 'receipt.txt'
    $errors = Join-Path $testRoot 'errors.txt'
    $process = Start-Process -FilePath (Join-Path $installRoot 'Drawer.exe') -ArgumentList @('--smoke-test', ('"' + $receipt + '"')) -WorkingDirectory $testRoot -WindowStyle Hidden -RedirectStandardError $errors -PassThru
    if (-not $process.WaitForExit(60000)) { $process.Kill(); $process.WaitForExit(); throw '설치 앱 기능 검증 시간 초과' }
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
    if ($smokeResult -notmatch 'mouse_painting=true') { throw '마우스 획/지우개/취소 시작 시험 실패' }
    if ($smokeResult -notmatch 'pointer_settings=true' -or $smokeResult -notmatch 'native_pen=.+') { throw '장치별 입력 설정/네이티브 초기화 시험 실패' }
    if ($smokeResult -notmatch 'pressure_mapping=true') { throw '필압 연결/설정 재열기 시작 시험 실패' }
    if ($smokeResult -notmatch 'pressure_curve=true') { throw '필압 범위/곡선 시작 시험 실패' }
    if ($smokeResult -notmatch 'synthetic_pen_attributes=true') { throw '가상 펜 속성/픽셀 적용 시작 시험 실패' }
    if ($smokeResult -notmatch 'mouse_origin_backend=true' -or $smokeResult -notmatch 'synthetic_pen_dedup=true') { throw '마우스 출처 경로/중복 제거 시작 시험 실패' }
    if ($smokeResult -notmatch 'missing_pen_properties=true') { throw '필압/기울기 누락 기본 브러시 시작 시험 실패' }
    if ($smokeResult -notmatch 'virtual_input_replay=true') { throw '가상 입력 재생/재현성/취소 시작 시험 실패' }
    if ($smokeResult -notmatch 'backend_switching=true' -or $smokeResult -notmatch 'wintab_initialization=Requested Wintab; active (Wintab|Pointer|Mouse)') { throw 'Pointer/Wintab/마우스 전환 및 복귀 시작 시험 실패' }
    if ($smokeResult -notmatch 'pen_click_controls=true') { throw '펜 임계값/상대 좌표/번호 버튼 픽셀 시험 실패' }
    if ($smokeResult -notmatch 'tablet_mapping=true') { throw '태블릿 영역/누락/영역 이탈 픽셀 시험 실패' }
    if ($smokeResult -notmatch 'local_acquisition=true') { throw '로컬 취득/보조 프로세스 취소/임시 정리 시험 실패' }
    if ($smokeResult -notmatch 'touch_gestures=true') { throw '터치 제스처/펜 우선/포커스 취소 시험 실패' }
    if ($smokeResult -notmatch 'controller_replay=true' -or $smokeResult -notmatch 'native_dial=.+') { throw '다이얼 명령/네이티브 상태 시작 시험 실패' }
    if ($smokeResult -notmatch 'keyboard_precision=true') { throw '키보드 이동/직선/농도/도구 전환 픽셀 시험 실패' }
    if ($smokeResult -notmatch 'brush_material=true') { throw '혼색/번짐/수채/알파 검증 실패' }
    if ($smokeResult -notmatch 'brush_presets=true') { throw '로컬 브러시 프리셋/크기 목록 시험 실패' }
    if ($smokeResult -notmatch 'stroke_ends=true') { throw '선 보정/시작·끝/실제 필압 분리 시험 실패' }
    if ($smokeResult -notmatch 'stroke_spacing=true') { throw '빠른 획/납작한 팁 보간 시험 실패' }
    if ($smokeResult -notmatch 'basic_brushes=true') { throw '기본 브러시/누적/이력 시험 실패' }
    if ($smokeResult -notmatch 'brush_parameters=true') { throw '브러시 수치/단위/위젯 시험 실패' }
    if ($smokeResult -notmatch 'mouse_virtual_pressure=true') { throw '마우스 가상 필압/재계산/예측 분리 시험 실패' }
    if ($smokeResult -notmatch 'textures_psd=true') { throw '브러시/종이 질감·PSD 베이크 시험 실패' }
    if ($smokeResult -notmatch 'extended_brushes=true' -or $smokeResult -notmatch 'embedded_tips=7') { throw '마커·번짐·픽셀 펜 실행 시험 실패' }
    if ($smokeResult -notmatch 'tip_profile=true') { throw '최소 굵기·농도/팁 모양 시험 실패' }
    if ($smokeResult -notmatch 'stroke_filter=true') { throw '독립 위치/필압 평활 시험 실패' }
    if ($smokeResult -notmatch 'tip_texture=true') { throw '팁별 질감·반전·수채 종료 경계 시험 실패' }
    if ($smokeResult -notmatch 'line_references=true') { throw '참조 경계·벡터 자석·다층 지우기 시험 실패' }
    if ($smokeResult -notmatch 'tonal_locks=true') { throw '닷지·번 톤 구간과 설정 잠금·복원 시험 실패' }
    if ($smokeResult -notmatch 'color_selector=true') { throw '색상환·색상 입력·색 이력 검증 실패' }
    if ($smokeResult -notmatch 'color_tables_mixing=true') { throw '로컬 색상표·혼색판·시험 이력 검증 실패' }
    if ($smokeResult -notmatch 'related_colors=true') { throw '중간색·유사색 격자·키보드 선택 검증 실패' }
    if ($smokeResult -notmatch 'transparency_fill_wand=true') { throw '채우기·마술봉 투명도 독립 판정 검증 실패' }
    if ($smokeResult -notmatch 'selection_shortcuts=true') { throw '선택 바로가기·윤곽 변환 검증 실패' }
    if ($smokeResult -notmatch 'magnetic_lasso=true') { throw '마그네틱 올가미·바깥 삭제 검증 실패' }
    if ($smokeResult -notmatch 'extended_fill=true') { throw '확장 채우기·애니메이션 출력 프레임 검증 실패' }
    if ($smokeResult -notmatch 'auto_shape=true') { throw '획 유지 도형 보정·원래 획 복귀 검증 실패' }
    if ($smokeResult -notmatch 'special_rulers=true') { throw '특수 자·레이어 연결·자 따라 그리기 검증 실패' }
    if ($smokeResult -notmatch 'alignment_detail=true') { throw '세부 정렬·개체 스냅 검증 실패' }
    if ($smokeResult -notmatch 'layer_management=true') { throw '레이어 생성·복제·순서·식별자 검증 실패' }
    if ($smokeResult -notmatch 'document_color=true' -or $smokeResult -notmatch 'gpu_color=true') { throw '문서 정밀도·RGBA32F GPU 비교 검증 실패' }
    if ($smokeResult -notmatch 'paper_background=true') { throw '종이 배경·비출력 체크색 검증 실패' }
    if ($smokeResult -notmatch 'expression_thresholds=true') { throw '표현색 임계값·실제 변환 검증 실패' }
    if ($smokeResult -notmatch 'layer_appearance=true') { throw '레이어 표시 효과 검증 실패' }
    if ($smokeResult -notmatch 'raster_conversion=true') { throw 'Raster/content conversion verification failed' }
    if ($smokeResult -notmatch 'file_objects=true') { throw 'Local file object verification failed' }
    if ($smokeResult -notmatch 'advanced_materials=true') { throw '소재 반복·변형·참조 선택·전체 갱신 검증 실패' }
    if ($smokeResult -notmatch 'material_layers=true') { throw 'Editable material layer verification failed' }
    if ($smokeResult -notmatch 'layer_defaults=true') { throw '레이어 종류별 생성 기본값 검증 실패' }
    if ($smokeResult -notmatch 'layer_roles=true') { throw '참조층 및 밑그림 출력 정책 검증 실패' }
    if ($smokeResult -notmatch 'layer_search=true') { throw '레이어 검색 및 직접 선택 검증 실패' }
    if ($smokeResult -notmatch 'adjustment_layers=true') { throw '비파괴 보정 레이어 검증 실패' }
    if ($smokeResult -notmatch 'layer_folders=true') { throw '레이어 폴더·회색조 마스크 검증 실패' }
    if ($smokeResult -notmatch 'layer_maintenance=true') { throw '보이는 층 복제·빈/숨김층 정리 검증 실패' }
    if ($smokeResult -notmatch 'individual_merge=true') { throw '원본 픽셀·마스크 개별 결합 검증 실패' }
    if ($smokeResult -notmatch 'layer_merge=true') { throw '레이어 병합·전체 합치기 검증 실패' }
    if ($smokeResult -notmatch 'linked_layers=true' -or $smokeResult -notmatch 'layer_movement=true') { throw 'Linked layer and movement verification failed' }
    if ($smokeResult -notmatch 'transfer_down=true') { throw 'Content transfer verification failed' }
    if ($smokeResult -notmatch 'layer_masks=true') { throw 'Layer mask verification failed' }
    if ($smokeResult -notmatch 'layer_comps=true') { throw 'Layer comps verification failed' }
    if ($smokeResult -notmatch 'extended_blend=true') { throw 'Extended layer blend verification failed' }
    if ($smokeResult -notmatch 'layer_blend=true') { throw '레이어 기본 혼합 모드 검증 실패' }
    if ($smokeResult -notmatch 'layer_clipping=true') { throw '알파 보호·클리핑·폴더 단독 표시 검증 실패' }
    if ($smokeResult -notmatch 'layer_properties=true') { throw '레이어 불투명도·잠금·구분색 검증 실패' }
    if ($smokeResult -notmatch 'layer_selection=true') { throw '레이어 알파·벡터 선택·윤곽 숨김과 칠 제한 검증 실패' }
    if ($smokeResult -notmatch 'drawing_snapping=true') { throw '스냅 우선순위·확대·수정키 복원 검증 실패' }
    if ($smokeResult -notmatch 'object_alignment=true') { throw '개체 정렬·균등 간격·한 취소 검증 실패' }
    if ($smokeResult -notmatch 'vector_creation=true') { throw '벡터 직선·곡선 생성 검증 실패' }
    if ($smokeResult -notmatch 'vector_edit=true') { throw '벡터 제어점·폭·색·필압 편집 검증 실패' }
    if ($smokeResult -notmatch 'vector_erase=true') { throw '벡터 선택·삭제·지우기 모드/참조/전체 레이어 검증 실패' }
    if ($smokeResult -notmatch 'vector_nodes=true') { throw '벡터 제어점 추가·삭제·각진/부드러운 접선 검증 실패' }
    if ($smokeResult -notmatch 'vector_refine=true') { throw '벡터 선 연결·단순화·밀기·다시 그리기 검증 실패' }
    if ($smokeResult -notmatch 'vector_tip=true') { throw '벡터 팁 변경·경로 표시 검증 실패' }
    if ($smokeResult -notmatch 'vector_pressure_reset=true') { throw '벡터 선택·전체 필압 초기화 검증 실패' }
    if ($smokeResult -notmatch 'vector_scope=true') { throw '벡터 기준점·범위 복사·선택 이력 검증 실패' }
    if ($smokeResult -notmatch 'path_draft=true') { throw '작성 중 경로 이력·확정·버리기·계속 작성 검증 실패' }
    if ($smokeResult -notmatch 'vector_cleanup=true') { throw '래스터 윤곽 변환·짧은 선/구간 정리·다른 속성 선 연결 검증 실패' }
    if ($smokeResult -notmatch 'affine_transform=true') { throw '기준점·축별 배율·회전·선택 픽셀 변형 검증 실패' }
    if ($smokeResult -notmatch 'tone_mapping=true') { throw 'RGB·채널 톤 커브·그라데이션 맵 검증 실패' }
    if ($smokeResult -notmatch 'additional_color=true') { throw '레벨·색균형·계조·이진화·반전 검증 실패' }
    if ($smokeResult -notmatch 'mesh_puppet=true') { throw '메시·퍼펫 핀 변형 검증 실패' }
    if ($smokeResult -notmatch 'raster_deformation=true') { throw '자유·원근·유동화 변형 검증 실패' }
    if ($smokeResult -notmatch 'transform_review=true') { throw '변형 품질·저장 확정/취소 검증 실패' }
    if ($smokeResult -notmatch 'effect_source_delete=true') { throw '효과 원본 삭제 검토/보존 검증 실패' }
    if ($smokeResult -notmatch 'text_extensions=true') { throw '앱 글꼴/원형 텍스트 검증 실패' }
    if ($smokeResult -notmatch 'page_management=true') { throw 'Page management verification failed' }
    if ($smokeResult -notmatch 'animation_export=true') { throw 'Animation output verification failed' }
    if ($smokeResult -notmatch 'light_table=true') { throw 'Light table verification failed' }
    if ($smokeResult -notmatch 'audio_device=.+') { throw 'Audio device report missing' }
    if ($smokeResult -notmatch 'timeline_audio=true') { throw 'Timeline audio verification failed' }
    if ($smokeResult -notmatch 'camera_2d=true') { throw '2D camera verification failed' }
    if ($smokeResult -notmatch 'onion_skin=true') { throw 'Onion skin verification failed' }
    if ($smokeResult -notmatch 'layer_keyframes=true') { throw 'Layer keyframe verification failed' }
    if ($smokeResult -notmatch 'timeline_timing=true') { throw 'Timeline timing verification failed' }
    if ($smokeResult -notmatch 'animation_timeline=true') { throw 'Animation timeline verification failed' }
    if ($smokeResult -notmatch 'shot_exchange=true') { throw 'Shot exchange verification failed' }
    if ($smokeResult -notmatch 'epub_metadata=true') { throw 'EPUB metadata verification failed' }
    if ($smokeResult -notmatch 'book_production=true') { throw 'Book production verification failed' }
    if ($smokeResult -notmatch 'page_structure=true') { throw 'Page structure verification failed' }
    if ($smokeResult -notmatch 'raster_books=true') { throw 'Raster book verification failed' }
    if ($smokeResult -notmatch 'comic_composition=true') { throw 'Comic composition verification failed' }
    if ($smokeResult -notmatch 'story_text=true') { throw '작품 텍스트 치환/연결 이력 검증 실패' }
    if ($smokeResult -notmatch 'text_typeset=true') { throw '고급 텍스트 조판/도구 동작 검증 실패' }
    if ($smokeResult -notmatch 'text_version=true') { throw '텍스트 변형/레이아웃 전환 검증 실패' }
    if ($smokeResult -notmatch 'font_library=true') { throw '로컬 글꼴 목록/문자표 검증 실패' }
    if ($smokeResult -notmatch 'text_format=true') { throw '글자군 서식/혼합 글꼴 검증 실패' }
    if ($smokeResult -notmatch 'text_reading=true') { throw '세로쓰기/자간/합자/루비 검증 실패' }
    if ($smokeResult -notmatch 'text_ime=true') { throw '캔버스 한글 조합/대체 글꼴 검증 실패' }
    if ($smokeResult -notmatch 'text_layout=true') { throw '텍스트/글꼴/정렬/줄 간격 검증 실패' }
    if ($smokeResult -notmatch 'quick_filters=true') { throw '필터 입출력/새 레이어/손실 확인 검증 실패' }
    if ($smokeResult -notmatch 'cleanup_filters=true') { throw '톤/먼지/틈 채움/선택 도구 검증 실패' }
    if ($smokeResult -notmatch 'filter_options=true') { throw '필터 방향/파형/경계/프리셋 검증 실패' }
    if ($smokeResult -notmatch 'finishing_filters=true') { throw '노멀/극좌표/스무딩 필터 검증 실패' }
    if ($smokeResult -notmatch 'transform_policy=true') { throw '선폭/기준점/메시 재편집 검증 실패' }
    if ($smokeResult -notmatch 'transform_handles=true') { throw '변형 핸들/각도 맞춤 검증 실패' }
    if ($smokeResult -notmatch 'metallic_material=true') { throw '금속 재질 레이어 검증 실패' }
    if ($smokeResult -notmatch 'local_inference=true') { throw '로컬 모델 추론 검증 실패' }
    if ($smokeResult -notmatch 'color_deepen=true') { throw '색 심화 픽셀/보정층·클리핑 미리보기 검증 실패' }
    if ($smokeResult -notmatch 'local_color=true') { throw '로컬 색 분포·광원/그림자 보정층 검증 실패' }
    if ($smokeResult -notmatch 'batch_filters=true') { throw '복수 레이어 필터·전체 확정/취소 검증 실패' }
    if ($smokeResult -notmatch 'warp_filters=true') { throw '왜곡 필터·투명 표본·미리보기·취소 검증 실패' }
    if ($smokeResult -notmatch 'surface_effects=true') { throw '공간 효과·순서·마스크·미리보기·취소 검증 실패' }
    if ($smokeResult -notmatch 'stylized_effects=true') { throw '색수차·필름·연필 효과 검증 실패' }
    if ($smokeResult -notmatch 'pixel_effects=true') { throw '모자이크·시드 노이즈·먼지/선폭 보정 검증 실패' }
    if ($smokeResult -notmatch 'detail_filters=true') { throw '샤픈·언샤프·명도/색 노이즈 제거 검증 실패' }
    if ($smokeResult -notmatch 'spatial_blur=true') { throw '공간 블러·조리개·비동기 취소·설정 저장 검증 실패' }
    if ($smokeResult -notmatch 'gaussian_blur=true') { throw '가우시안 블러·투명 경계·반경0·저장/취소 검증 실패' }
    if ($smokeResult -notmatch 'svg_io=true') { throw '로컬 SVG 가져오기·내보내기·복사 검증 실패' }
    if ($smokeResult -notmatch 'shape_creation=true') { throw '도형·곡선·래스터/벡터·입력 취소 검증 실패' }
    if ($smokeResult -notmatch 'gradient_ramps=true') { throw '그라데이션 색상 지점·혼색·밝기·프리셋 검증 실패' }
    if ($smokeResult -notmatch 'gradient_tools=true') { throw '그라데이션·자유형 영역·편집 레이어 검증 실패' }
    if ($smokeResult -notmatch 'enclosed_fill=true') { throw '올가미·닫힌 영역·작은 영역 보정 검증 실패' }
    if ($smokeResult -notmatch 'fill_references=true') { throw '참조 집합·제외·확장·좁은 영역 검증 실패' }
    if ($smokeResult -notmatch 'selection_border=true') { throw '선택 테두리 정렬·구멍·마스크 보존 검증 실패' }
    if ($smokeResult -notmatch 'color_range=true') { throw '색상 범위 선택·허용치·참조 검증 실패' }
    if ($smokeResult -notmatch 'quick_mask=true') { throw '퀵 마스크 표시·편집·저장 레이어 검증 실패' }
    if ($smokeResult -notmatch 'selection_filter=true') { throw '선택 확대·축소·경계 흐림 검증 실패' }
    if ($smokeResult -notmatch 'selection_geometry=true') { throw '타원·다각형 선택·합성·취소 검증 실패' }
    if ($smokeResult -notmatch 'drawing_rulers=true') { throw '그리기 자·대칭·원근·임시 스냅 검증 실패' }
    if ($smokeResult -notmatch 'selection_edit=true') { throw '마술봉·선택 브러시·강도/취소 검증 실패' }
    if ($smokeResult -notmatch 'bucket_fill=true') { throw '영역·선택 채우기 및 미리보기 취소 검증 실패' }
    if ($smokeResult -notmatch 'selection_transfer=true') { throw '선택 이동·복사·새 레이어 붙여넣기 검증 실패' }
    if ($smokeResult -notmatch 'selected_paint=true') { throw '선택 강도·RGBA덮어쓰기·그리기/지우기 검증 실패' }
    if ($smokeResult -notmatch 'page_selections=true') { throw '페이지 선택·마우스/펜 취소·셀/문서 재열기 검증 실패' }
    if ($smokeResult -notmatch 'soft_proof=true') { throw 'ICC 보기·톤/레벨·적용/취소/재시작 검증 실패' }
    if ($smokeResult -notmatch 'screen_sampling=true') { throw '화면색 평균·창복원·참조레이어 검증 실패' }
    if ($smokeResult -notmatch 'hsl_swatches=true') { throw 'HSL·색상표 이동/복제/복원 검증 실패' }
    if ($smokeResult -notmatch 'color_modes=true') { throw '주색·보조색·투명색 지우기·동시 해제 검증 실패' }
    if ($smokeResult -notmatch 'brush_color=true') { throw 'CIELAB·밝기·두 잉크 합성 시험 실패' }
    if ($smokeResult -notmatch 'brush_behavior=true') { throw '화면 크기·간격 농도·연속 분사 시험 실패' }
    if ($smokeResult -notmatch 'work_time=true') { throw '문서별 작업시간·초기화·로컬 복구 검증 실패' }
    if ($smokeResult -notmatch 'timelapse=true') { throw '로컬 타임랩스·일시정지·출력·종료 복원 검증 실패' }
    if ($smokeResult -notmatch 'timesheet_exchange=true') { throw '자체 타임시트·공유 셀 교환 검증 실패' }
    if ($smokeResult -notmatch 'animation_delivery=true') { throw 'GIF/WebP·출력 옵션·스냅샷·로컬 인코더 없음 대안 시험 실패' }
    if ($smokeResult -notmatch 'tool_slots=true') { throw '농담·섬유·슬롯·오프라인 묶음 시험 실패' }
    if ($smokeResult -notmatch 'tool_modes=true') { throw '픽셀 덮어쓰기·알파 유지·수채 공급 시험 실패' }
    if ($smokeResult -notmatch 'user_tools=true') { throw '독립 사용자 도구·분류·그룹·참조 복원 시험 실패' }
    if ($smokeResult -notmatch 'velocity_pressure=true') { throw '속도 크기·농도·장치 필압·시각 경계 시험 실패' }
    if ($smokeResult -notmatch 'pixel_eyedropper=true') { throw '픽셀 펜·RGB/RGBA 스포이트·색 이력 시험 실패' }
    if ($smokeResult -notmatch 'brush_composite=true') { throw '브러시·폴더 합성·필압 혼색·저장 시험 실패' }
    if ($smokeResult -notmatch 'abr_import=true') { throw 'ABR 식별·변환·저장·입력 시험 실패' }
    if ($smokeResult -notmatch 'image_materials=true') { throw '로컬 이미지 소재 등록·픽셀 적용 시험 실패' }
    if ($smokeResult -notmatch 'clone_stamp=true') { throw '복제 원점·원본 스냅샷 복제 시험 실패' }
    if ($smokeResult -notmatch 'brush_variation=true') { throw '색상 변동·두 팁 합성 시험 실패' }
    if ($smokeResult -notmatch 'brush_pattern=true') { throw '산포·리본·장식 반복 시험 실패' }
    if ($smokeResult -notmatch 'brush_dynamics=true') { throw '방향·입력별 크기 곡선 시험 실패' }
    if ($smokeResult -notmatch 'tip_sequence=true') { throw '팁 순서·혼색 방식 시험 실패' }
    if ($smokeResult -notmatch 'pigment_independent=true') { throw '독립 혼색/희석/알파 제어 시험 실패' }
    Get-Content -LiteralPath $receipt
    Write-Output '설치·실행·설정 저장·재설치·제거 시험 통과; 개발도구 PATH 없이 시작; 사용자 파일 18개 보존.'
} finally {
    if ($null -ne $process -and -not $process.HasExited) { $process.Kill(); $process.WaitForExit() }
    $env:DRAWER_DATA_DIR = $oldData
    $env:PATH = $oldPath
    $env:DRAWER_SMOKE_DIAGNOSTICS = $oldDiagnostics
    $resolvedTest = [IO.Path]::GetFullPath($testRoot)
    if ($resolvedTest -ne (Join-Path $projectRoot '.installation-test')) { throw '시험 정리 경로 불일치' }
    if ((Get-Item -LiteralPath $resolvedTest).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw '시험 정리 대상은 링크일 수 없습니다.' }
    Remove-Item -LiteralPath $resolvedTest -Recurse -Force
}
