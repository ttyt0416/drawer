# Drawer 로컬 도움말

## welcome | 시작 안내 | 도움말 처음 시작 로컬 오프라인

Drawer는 인터넷 연결과 계정 없이 로컬 문서와 설정을 사용하는 그림 작업공간입니다.
New canvas로 그림을 만들고 Open으로 자체 문서를 엽니다. 현재 문서에 직접 붓질하는 기능은 아직 제공하지 않습니다. 도구 시험은 독립 Test painting 창에서 할 수 있습니다.
검색란에 한국어 용어, 영어 메뉴 이름 또는 현재 지정한 단축키를 입력합니다. 검색 결과가 없으면 검색어를 지우거나 시작 안내로 돌아갑니다.

[문서와 파일](local:files)
[도구와 시험 칠하기](local:tools)
[실제 단축키 목록](local:shortcuts)
[버전 정보](local:version)
[라이선스 고지](local:licenses)

## files | 문서·저장·가져오기 | 파일 저장 열기 PNG JPEG TIFF 클립보드 취소

New canvas는 독립 미저장 문서를 만듭니다. px/mm/inch, DPI, 비율 잠금과 로컬 크기 프리셋을 선택합니다.
Save는 편집 가능한 .drawer 형식으로 저장합니다. Save as는 새 위치로 저장하고 이후 저장 대상을 그 위치로 바꿉니다. PNG export는 합성된 그림을 내보내며 자체 문서와 보기 상태를 바꾸지 않습니다.
Image as new document는 PNG/JPEG/TIFF를 미리 본 뒤 독립 그림으로 가져옵니다. New from clipboard는 이미지 클립보드로 새 그림을 만들고 Paste image layer는 이동 가능한 새 레이어를 미리 본 뒤 한 번에 확정합니다. 취소는 기존 그림을 유지합니다.
Recent files는 최근 성공한 경로를 보존합니다. 원본 이미지에 덮어쓰지 않습니다. 마지막 보기를 닫거나 종료할 때 미저장 그림의 저장·버리기·취소를 선택합니다.
Local file actions의 저장본 되돌리기는 변경을 버린다는 확인 뒤 실행합니다. 자체 파일 연결은 Windows 기본 앱 설정에서 직접 선택합니다.

[크기와 DPI](local:size)
[여러 보기와 모니터](local:view)
[영상 가져오기](local:movie)

## size | 크기·해상도·DPI | 캔버스 물리 크기 밀리미터 인치 비율 회전 자르기

Sizing에서 이미지 재샘플링, DPI만 변경, 캔버스 크기/각 변 여백 변경, 자르기, 좌우·상하 반전, 90도 회전을 선택합니다.
크기 창의 보간은 보기 보간과 별개입니다. 픽셀 유지 DPI 변경은 픽셀을 바꾸지 않습니다. 물리 크기 유지는 DPI에 맞게 픽셀을 재샘플링합니다.
미리보기에서 적용하거나 취소합니다. 내용 제거는 확인을 거치고 확정된 변경은 Undo 한 번으로 복원합니다. 캔버스 밖 레이어 픽셀도 자체 파일에 보존됩니다.

[문서와 파일](local:files)

## view | 보기·내비게이터·모니터 | 확대 축소 줌 이동 배율 회전 반전 선택 화면 모니터

휠은 커서 위치를 유지하며 확대합니다. 가운데 버튼 또는 Space와 드래그는 보기를 이동합니다. Zoom region은 드래그 영역을 화면에 맞춥니다.
100%, 0°, Center, Reset view는 각각 배율·회전·위치·전체 보기를 복원합니다. 보기 반전은 저장 픽셀을 반전하지 않습니다.
Duplicate view는 같은 그림의 다른 배율/각도 탭을 만듭니다. Ctrl+PageUp/Down은 보기 순서를 이동하며 Alt+1부터 Alt+9는 현재 순서의 탭을 선택합니다. 없는 번호는 아무 작업도 하지 않습니다.
Navigator의 녹색 경계는 보이는 영역, 파란 채움과 점선은 선택입니다. 클릭은 보기 중심을 옮기며 그림을 편집하지 않습니다.
Display view는 선택 모니터의 별도 캔버스 창입니다. 보기 동기화를 끄면 독립 배율과 중심을 유지합니다. F11은 전체화면, Esc는 창 모드입니다. 화면이 사라지면 사용 가능한 화면으로 회수합니다.

[눈금·격자·가이드](local:aids)
[작업공간과 패널](local:workspace)

## aids | 눈금·격자·가이드 | 길이 인쇄 물리 크기 눈금 격자 가이드 보정

Canvas aids에서 단위와 눈금, 격자, 가이드 표시를 선택합니다. 격자 간격/원점과 가이드 추가는 문서 편집이며 Undo로 복원합니다. 표시 켜기/끄기는 설정입니다.
인쇄 크기를 보려면 기준선의 실제 밀리미터 길이를 입력해 화면을 보정합니다. 모니터를 바꾸면 다시 보정합니다.

[보기 조절](local:view)

## tools | 도구·시험 칠하기 | 브러시 붓 연필 지우개 워시 필압 펜 마우스 커서 색

Brushes는 Pen, Pencil, Eraser, Wash 팁과 현재 크기·타원 비율·팁 회전을 표시합니다. 커서는 보기 회전/반전/화면 배율을 반영한 외곽과 도구 아이콘입니다.
Test painting은 문서와 분리된 시험 영역입니다. 현재 도구·크기·색으로 드래그해 칠하고 Eraser로 지웁니다. 내용과 표시 상태는 로컬 저장되며 문서 Undo에 포함되지 않습니다.
펜 하드웨어 검증은 별도이며 마우스로 시험할 수 있습니다.
Tool input behavior에서 임시 도구 유지 시간과 실제 화면 픽셀의 드래그 거리를 선택합니다. 짧은 도구 키는 선택 유지, 기본 300ms 이상 누른 키는 놓으면 이전 도구로 돌아옵니다. 포커스/문서 변경은 유지 입력을 끝냅니다.

[단축키와 명령](local:shortcuts)
[참고 이미지와 색](local:reference)

## reference | 참고 이미지·색 선택 | 참고 서브뷰 색 스포이트 투명도 색환

References에 로컬 이미지 또는 이미지 클립보드를 추가합니다. 목록과 Previous/Next로 전환하고 휠/가운데 버튼/Space 드래그로 보기를 조절합니다.
Pick color 클릭 또는 Auto pick으로 원본 색과 알파를 읽습니다. Color ring은 커서 옆에 선택 색을 보여줍니다. Open as new drawing은 독립 새 문서를 만듭니다.
참고 복사본과 설정은 로컬 저장합니다. 참고 제거는 원본을 지우지 않습니다. Save set/Load set은 복사본을 포함하는 로컬 묶음이며 참고 설정에 별도 취소와 이전본 복원을 제공합니다.

[도구와 시험 칠하기](local:tools)

## workspace | 작업공간·패널 | 배치 도킹 분리 레이어 타임라인 설정 백업 초기화

Panels는 브러시/색/레이어/타임라인 표시, 썸네일과 도구 아이콘/이름/획 미리보기, 속성/명령막대 표시를 선택합니다. 대상 선택과 그림 데이터는 그대로 유지합니다.
Workspaces 또는 패널 Dock 메뉴에서 좌/우 도킹 또는 Floating 분리 창을 선택합니다. 이름 있는 배치를 저장하고 전환·갱신·삭제·재적용할 수 있습니다.
로컬 가져오기/재적용은 설정군을 따로 골라 확정합니다. 미선택 설정은 유지합니다. 적용 전 자동 백업과 미저장 배치의 별도 이름 저장은 따로 선택합니다. 취소는 현재 설정을 유지합니다.
Interface size는 모니터 배율에 더하는 50–200% 크기입니다. 그림의 픽셀과 보기 중심은 유지합니다.

[단축키·빠른 접근·원형 메뉴](local:shortcuts)

## shortcuts | 실제 단축키·명령 배정 | 키보드 shortcut 빠른 접근 원형 메뉴 순환 장치

아래 실제 목록은 현재 설정의 키 배정입니다. 기본 키를 바꿨다면 바꾼 배정을 보여줍니다.
Input and commands에서 키와 명령을 배정합니다. 중복/예약 키는 즉시 알리고 기존 배정을 덮지 않습니다. 변경은 임시본에서 확정하며 취소는 기존 배정을 유지합니다.
도구 순환 목록은 이름·순서·역순·복제·삭제와 다음/이전 입력을 지원합니다. 삭제된 도구는 건너뛰고 빈 목록은 이유를 표시합니다.
빠른 접근/명령막대는 명령·도구·고정색·구분자, 검색과 타일/목록/팝업을 지원합니다. 일반 드래그 또는 선택한 Ctrl 필요 드래그로 재배열합니다. 등록한 고정색은 현재 색 변경에 영향받지 않습니다.
원형 메뉴의 4/8방향 슬롯은 마우스와 1–8 키, 방향키로 같은 명령을 선택합니다. 중앙/Esc/영역 밖은 취소입니다. 공통 명령 선택은 기존 배정을 선택해 열고 Confirm으로만 임시본을 바꿉니다.
장치 어댑터가 제공하는 이름 있는 동작 또는 포인터 추가 버튼을 같은 명령에 배정합니다. 사용할 수 없는 동작은 이유를 표시하고 키보드 대체를 사용합니다.

[도구 유지 시간](local:tools)

## movie | 로컬 영상 가져오기 | 애니메이션 영상 동영상 프레임 셀 오디오 FFmpeg

Movie 메뉴는 로컬 영상으로 새 애니메이션을 만들거나 현재 애니메이션에 별도 트랙으로 삽입합니다. 로컬 FFmpeg/FFprobe 7.1이 필요합니다. Choose local FFmpeg로 직접 지정하며 앱은 코덱을 내려받지 않습니다.
시간 범위·FPS·크기·보간·오디오를 선택하고 Decode range and preview에서 확인한 뒤 확정합니다. 대응표는 목표 FPS의 전체 프레임 위치를 표시합니다.
오디오는 채널 수를 유지한 48kHz PCM입니다. 취소는 현재 그림을 유지하며 삽입은 Undo 한 번으로 제거합니다. Frame 슬라이더는 확인 프레임을 바꾸며 그림을 편집하지 않습니다.
지원 범위는 MOV/MP4 H.264, MKV FFV1, AVI rawvideo, WebM VP9의 검사된 8bit SDR 변형입니다. 자세한 제약은 설치 폴더의 MOVIE-IMPORT.md를 읽습니다.

[문서와 파일](local:files)

## version | 버전 정보 | 버전 about Windows Drawer

아래 버전은 실행 중인 Drawer의 버전입니다. 로컬 문서와 도움말, 기본 도구 및 고지는 실행 파일/설치 묶음에서 제공합니다. 계정이나 네트워크 연결을 요구하지 않습니다.

[라이선스 고지](local:licenses)

## licenses | 라이선스·구성요소 고지 | 저작권 라이선스 license MIT Apache BSD copyright 구성요소

아래에는 빌드에 사용한 외부 구성요소의 로컬 고지 전체를 표시합니다. 읽기와 검색은 인터넷을 사용하지 않습니다. 배포 폴더의 THIRD-PARTY-NOTICES.txt와 COMPONENTS.json에서도 읽을 수 있습니다.
기본 팁/아이콘은 자체 코드로 제작했습니다. 기본 UI 글꼴은 묶음 고지의 내장 글꼴이며, 한국어 표시에 Windows에 설치된 맑은 고딕을 사용할 수 있습니다. 설치된 시스템 글꼴 파일은 이 배포에 넣지 않습니다.

[버전 정보](local:version)
[시작 안내](local:welcome)

## behavior | 패널·커서·가장자리·절전 | 자동 숨김 스크롤바 커서 화면 가장자리 절전 전원

Panels, cursor and power에서 브러시/색/레이어/타임라인 패널의 유휴 자동 숨김, 스크롤바와 브러시 커서 표시를 선택합니다. 숨긴 패널은 제목/내용 표시 버튼에 마우스를 가져가거나 키보드로 접근해 다시 펼칩니다. 분리 창은 원래 전체 크기를 유지한 채 접고 펼칩니다.
화면 가장자리 이동을 켜면 캔버스의 primary 버튼을 누르는 동안 가장자리 방향으로 보기가 이동합니다. 여백과 속도는 화면의 실제 픽셀 기준입니다. 버튼 해제·포커스 상실은 이동을 끝냅니다. 보기는 저장 픽셀을 바꾸지 않습니다.
자동 유휴 절전 억제는 기본 꺼짐입니다. 켜면 칠하기·영상/이미지/크기 처리와 저장/PNG 출력 중에만 요청하며 완료·실패·취소·종료에서는 해제합니다. 화면 켜짐 유지도 따로 고를 수 있습니다. 사용자가 직접 요청한 절전은 Windows 정책을 따릅니다.
설정은 임시본에서 확정하고 취소는 현재 값을 유지합니다. 적용 전 백업·초기값·이전본을 제공합니다.

[작업공간과 패널](local:workspace)
[보기 조절](local:view)

## cursors | 원시·보정·예측 커서 | 커서 안정화 보정 예측 최소 크기 방향 반전 기울기

Cursor styles에서 각 도구의 외곽/십자/아이콘/시스템 커서와 작은 외곽의 최소 화면 픽셀 크기를 선택합니다. 팁 회전과 보기 회전/반전을 함께 반영합니다.
Raw는 현재 수신 좌표를 흰색으로 표시하고 Corrected는 실제 획 계산에 쓰는 필터 좌표를 청록색으로 표시합니다. Both는 원시 십자와 보정 외곽을 구분합니다. 예측은 주황색 열린 표시이며 확정 픽셀에 사용하지 않습니다. 시스템 커서는 원시 마우스 위치입니다.
안정화 강도는 Tool input behavior의 도구별 설정입니다. 커서 표시 선택은 안정화나 결과 픽셀을 바꾸지 않습니다. 시험 칠하기는 같은 필터를 사용하며 입력 이탈·포커스 상실·버튼 해제는 오래된 표시를 끝냅니다.

[도구·시험 칠하기](local:tools)

## information | 좌표·길이·색 정보와 테마 | 정보 좌표 거리 측정 단위 RGB HSV HSL CMYK 테마

Information은 포인터 또는 측정점의 문서 좌표, 두 점의 가로/세로 차이와 직선 거리를 보여줍니다. Pick start/Pick end를 누른 뒤 캔버스를 클릭하거나 선택한 점의 숫자를 조절합니다. 측정선과 위치는 세션 상태이며 그림을 편집하지 않습니다.
표시 단위는 Document(현재 눈금 단위), px, mm, cm, inch, point입니다. 실제 길이는 문서 DPI를 사용하며 보기 확대/회전/반전에 영향받지 않습니다.
RGB·알파·HSV/HSL을 고르고 CMYK는 Choose local CMYK output profile에서 선택한 ICC 출력 프로파일로 계산합니다. 프로파일은 최대 4MiB의 로컬 파일만 읽고 수정하지 않습니다. 알맞은 프로파일이 없거나 읽을 수 없으면 CMYK를 계산하지 않고 이유를 표시합니다. 값은 현재 sRGB 문서의 합성 픽셀을 기준으로 하며 출력 변환은 8bit 채널 값입니다.
Dark/Light 테마와 도킹된 도구 패널의 좌/우 배치를 선택하고 Interface size에서 UI 배율을 조절합니다. 표시/단위/색 형식/프로파일 경로는 로컬 설정에 저장하고 변경 전 백업합니다.

[눈금·보정](local:aids)
[작업공간](local:workspace)

## modifier-tools | Modifier tools | Ctrl Shift Alt temporary hold conflicts 수정키 임시 도구 충돌

Open Tool input behavior to assign Ctrl, Shift or Alt to a temporary tool. Modifier tools always return to the previous tool when released, even before the short-key threshold. Letter tool keys still follow the configured hold threshold. Overlapping modifiers restore in press order.

Shared shortcuts appear in the editor. Key chords, input fields, menus and dialogs take priority and suspend modifier tools until the modifier is released. Release and press again after closing a popup. Window focus loss cancels pending tool input. These assignments are local workspace preferences and do not add drawing history.

## screen-mode | Simple and full screens | F12 panels screen mode 간단 전체 도구 화면

The screen button switches between all panels and a simple drawing screen. F12 is the default shortcut on a new profile; assign Simple/full screen in Input and commands for an existing profile. Simple mode keeps document tabs, tools and canvas controls, and provides All tools and commands for the remaining controls. Floating tool panels and the navigator return to their previous layout when switching back.

Screen mode is session-only. It preserves document contents, selections, tool choices, view values and pending previews. The screen button remains available while a preview disables drawing commands. Independently opened information, reference and test-painting windows remain usable.

## input-editing | 입력란 편집과 상황 도움말 | 잘라내기 복사 붙여넣기 실행취소 다시실행 삭제 전체선택 숫자 Enter Esc 한글 IME F1

도움말 키는 현재 포커스가 있는 제어기의 주제를 우선합니다. 포커스가 없으면 포인터 아래 제어기/캔버스 주제를 열고, 해당 주제가 없으면 시작 안내를 엽니다. 모든 내용은 프로그램 안에 포함되어 있습니다.

이름·숫자 입력란은 오른쪽 메뉴에서 실행취소/다시실행·잘라내기·복사·붙여넣기·선택 삭제·전체선택을 제공합니다. Ctrl+Z/Y와 Ctrl+X/C/V/A도 입력란에서 사용할 수 있습니다. 잘라내기는 클립보드에 쓰기가 성공해야 선택 문자를 지웁니다. 선택이 없으면 잘라내기/복사/선택 삭제는 아무것도 바꾸지 않습니다.

문자와 숫자는 Enter 또는 포커스 이동으로 확정합니다. 잘못된 숫자는 이유를 표시하고 이전 값을 유지합니다. Esc는 편집 시작값으로 복구하며 그림의 이력을 바꾸지 않습니다. 숫자 버튼을 끌면 범위 안에서 직접 조절하며 클릭/Tab으로 문자 편집을 시작합니다. 슬라이더 옆에도 같은 숫자 입력란이 있습니다. 한글 조합 중에는 조합을 마친 뒤 확정합니다. 입력한 값의 최종 문서 적용은 각 미리보기의 확정 버튼을 사용합니다.

## layout | 도크 잠금과 창 정렬 | 도크 너비 높이 위치 잠금 패널 탭 팝업 제목줄 메뉴줄 계단 바둑판

상단 Layout에서 좌우 도크의 실제 화면 픽셀 너비/높이, 크기와 도킹 잠금, 겹쳐 쌓기/탭/팝업 표시를 고릅니다. 분리 패널의 너비·높이·위치 잠금도 개별로 지정합니다. 위치 잠금은 머리말의 도킹 변경을 막고 분리 창의 위치를 유지합니다. 모니터가 없어지거나 화면 밖이면 기존 복구 규칙으로 회수합니다.

제목줄·명령 메뉴줄을 숨겨도 상단 Layout/화면 전환 버튼은 유지합니다. 숨긴 명령은 All tools and commands에서 사용할 수 있습니다. 계단/바둑판 정렬은 열린 탭마다 독립된 캔버스 보기창을 만들고 그림을 공유합니다. 보기창의 휠/이동은 해당 보기만 바꾸며 창을 닫을 때 마지막 그림의 저장 확인 규칙을 적용합니다. Tabs로 돌아오면 일반 캔버스를 사용합니다. 미리보기/문서 변경은 별도 확정 절차를 유지합니다.

설정은 임시본에서 적용/취소하고 작업공간에 실제 화면 픽셀로 저장합니다. DPI/UI 배율 변경 시 논리 픽셀로 다시 계산합니다. 도킹된 창은 화면이 작으면 캔버스 공간을 남기도록 표시 너비를 제한하며 저장한 잠금 너비는 보존합니다.

## tool-properties | 도구 속성 표시와 복사 | 속성 값 표시 숨김 슬라이더 숫자 표시기 일괄 복사 도구군 프리셋

Tool properties에서 네 브러시의 크기·비율·각도·안정화를 각각 표시/숨김하고 슬라이더·숫자·읽기 전용 표시기를 고릅니다. 도구를 바꾸면 해당 도구의 값과 표시 방식을 사용합니다. 기존 공통 설정은 처음에 도구별 설정으로 옮깁니다.

선택한 속성의 값만 또는 표시 상태만 같은 도구군/선택한 도구/전체 도구에 복사합니다. 미리보기의 대상·전후 값·미지원 속성을 확인한 뒤 Accept preview into draft로 임시본에 반영하고 Apply로 저장합니다. Cancel property copy나 설정 창 Cancel은 전체 대상을 보존합니다. 스포이트/이동 도구에는 브러시 공통 속성을 새로 만들지 않습니다. 현재 네 브러시의 공통 범위는 같으며 대상 정의의 범위가 좁으면 제한된 값을 미리보기에 표시합니다.

로컬 속성 프리셋 JSON 저장/읽기와 적용 전 자동 백업 복원을 제공합니다. 전체 초기화는 확인 미리보기를 거쳐 임시본에 반영하며 적용 전 설정은 백업으로 복구합니다. 도구 설정은 그림의 실행 취소와 분리됩니다. 스포이트/이동 도구 자체의 편집 기능은 해당 기능 항목에서 구현합니다.

## preferences | 언어·IME·확인창 | 한글 영어 입력기 자동 전환 조합 복원 경고 확인창 전체 개별

Language, IME and confirmations에서 로컬 튜토리얼/도구 설명 언어와 입력란의 IME 유지·열기·닫기 옵션을 고릅니다. 기본값은 사용자 상태 유지입니다. 숫자 입력란은 유지/닫기만 선택할 수 있습니다. 옵션은 Drawer UI 스레드의 포커스 창에만 적용하고 입력 이탈·창 포커스 상실 때 진입 전 IME 열림 상태를 복원합니다. 입력 언어나 변환 모드 자체를 바꾸지 않습니다.

조합 중에는 그림 단축키와 입력 Enter 확정을 억제합니다. Esc는 조합을 취소하며 입력 창은 유지합니다. 창 전환도 조합 문자열을 숫자로 확정하지 않습니다. 시스템이 상태 제어를 거부하면 이전 상태를 유지하고 이유를 표시합니다. IME에서 전달된 텍스트는 입력란의 세션에만 사용합니다.

숨긴 도구 속성/도크 초기화 확인창은 목록에서 각각 복원하거나 Restore all confirmations로 모두 복원합니다. 문서의 미저장 변경을 묻는 저장 확인창은 항상 표시됩니다. 초기화 확인에서 Do not show again을 확정하면 그 안내만 숨깁니다.

## tutorial-ko | 로컬 그리기 안내 | 튜토리얼 처음 마우스 로컬 문서 저장 펜

1. New canvas에서 크기와 DPI를 정하고 새 문서를 만듭니다.
2. B/N/E/W로 펜·연필·지우개·워시를 고르고 캔버스에 왼쪽 버튼을 눌러 그립니다. 한 번 놓을 때 한 획을 확정하며 Undo 한 번으로 되돌립니다. 지우개는 투명도를 지웁니다. Test painting은 문서와 분리된 시험 영역입니다.
3. 휠로 확대하고 가운데 버튼 또는 Space+드래그로 보기만 이동합니다. 정보 창에서 두 점을 지정하면 문서 단위로 거리를 측정할 수 있습니다.
4. Save as로 로컬 .drawer 문서를 저장하고 PNG 출력은 별도 사본으로 만듭니다. 여러 보기창은 같은 그림을 공유합니다.
5. 입력란은 Enter/포커스 이동으로 확정하고 Esc로 편집을 취소합니다. 조합 중 Esc는 조합만 취소합니다. F1은 포커스 제어기의 로컬 설명을 엽니다.
6. 펜 하드웨어 검증은 별도 단계입니다. 온라인 계정이나 클라우드 소재가 필요하지 않습니다.

## tutorial-en | Offline drawing tutorial | tutorial beginner mouse pen local save

1. Create a document with New canvas, dimensions and DPI.
2. Choose Pen/Pencil/Eraser/Wash with the default B/N/E/W keys. Draw on the canvas with the left mouse button. Release to commit one stroke; Undo restores it in one step. Eraser removes alpha. Test painting is a separate practice area.
3. Zoom with the wheel and pan with the middle button or Space+drag. The Information window measures two points in document units.
4. Save a local .drawer document; PNG export creates a separate copy. Multiple views share one drawing.
5. Commit field edits with Enter or focus change; Esc cancels an edit. During IME composition, Esc cancels composition only. F1 opens local context help.
6. Physical pen validation is a separate step. No online account or cloud assets are required.

## mouse-painting | 마우스 그리기와 지우기 | 마우스 획 취소 선택 지우개 알파

캔버스 왼쪽 누름부터 놓음까지 한 획입니다. 움직이지 않고 클릭해도 한 점을 찍고 빠르게 움직인 구간도 연결합니다. 현재 래스터와 선택 마스크 안에서만 그리며 선택이 비어 있으면 변경하지 않습니다. 획 도중 도구·색·크기를 바꾸어도 시작할 때의 값으로 끝냅니다.

누르는 동안 다른 보기에도 획을 미리 표시합니다. 정상적인 창 밖 놓음은 마지막 좌표까지 확정합니다. Esc·포커스 상실·포인터 취소·놓음 누락은 진행 중인 획을 버립니다. 저장/문서 닫기/화면 전환 명령은 유효한 진행 획을 먼저 확정합니다. 선택 제한과 지우기의 결과도 Undo/Redo로 복원합니다. 실제 펜 검증은 별도입니다.

## pointer-input | 마우스·펜 입력 구분 | 장치 종류 ID 안정화 펜 터치

Mouse and pen input에서 마우스와 펜의 그리기 허용·안정화 덮어쓰기를 각각 설정합니다. 덮어쓰기를 끄면 현재 도구의 안정화를 사용합니다. Apply는 적용 전 백업과 함께 로컬 저장하고 Cancel은 이전 값을 유지합니다. 마지막 그리기 종류·세션 장치/접촉 ID와 Windows 펜 연결 상태를 표시합니다.

처음 누른 장치가 놓거나 취소할 때까지 획을 소유합니다. 소유 중 다른 장치의 입력은 해당 획에 넣지 않으며 종료 후 새로 눌러야 그립니다. 마우스 ID 0은 창 라이브러리가 제공하는 통합 입력입니다. 종류 정보가 없는 터치는 압력 유무로 펜으로 추정하지 않으며 그리지 않습니다. 펜 ID는 세션 동안만 유효합니다. 연결 성공은 실제 펜 시험 완료를 의미하지 않습니다.

## pressure | 브러시 필압 연결 | 필압 크기 불투명도 독립 펜 마우스 프리셋

Brush pressure에서 펜·연필·지우개·워시 각각의 Pressure controls size와 Pressure controls opacity를 독립 선택합니다. 기본값은 크기 켬/불투명도 끔입니다. 연결된 축은 정규화 필압 0–1을 곱하고 연결하지 않은 축은 유지합니다. 크기 연결에서 0압력은 자국을 남기지 않으며 불투명도 연결만 켠 경우 굵기는 변하지 않습니다. 마우스와 필압 정보가 없는 장치는 1로 동작합니다.

획 도중 설정을 바꾸어도 시작값으로 끝냅니다. 불투명도 연결은 겹친 팁의 최대 기여도를 사용하여 같은 획이 왕복해도 압력보다 진해지지 않습니다. 지우개는 같은 비율로 알파를 제거합니다. 새 획은 기존 픽셀 위에 새로 합성합니다. 위치 보정과는 별도 계산입니다.

Apply/Cancel과 적용 전 백업을 제공하고 도구별 필압 연결을 로컬 JSON 프리셋으로 저장/불러옵니다. 기존 파일을 바꾸려면 Replace an existing preset을 명시적으로 선택합니다. 잘못된 버전·손상·64KiB 초과 프리셋은 임시 설정을 유지합니다. 실제 펜/드라이버의 압력 값 정확도 검증은 별도 대기입니다.

필압 입력 범위 Minimum/Maximum은 그리기 장치가 보고한 값 중 사용할 구간입니다. 최소값 아래는 0, 최대값 위는 1로 제한한 뒤 곡선을 적용합니다. 범위 안의 25%/50%/75%에서 출력 값을 편집하며 양끝은 0과 1로 고정합니다. 수평 구간은 허용하지만 감소하는 곡선·최소=최대·역전 범위는 이유를 표시하고 Apply/프리셋 저장을 막습니다. 그래프는 원시 입력 필압에 대한 최종 출력을 보여줍니다. 변경한 범위/곡선은 이후 시작하는 획에만 적용하고 기존 픽셀과 그림 이력은 바꾸지 않습니다.

마우스·펜 입력 설정에서 지원되는 기울기를 팁 각도에 연결하고, 펜 지우개 또는 보고된 버튼을 누른 채 시작한 획을 지우개로 그리도록 선택할 수 있습니다. 기울기는 보기 회전·반전을 반영한 문서 방향으로 변환합니다. 속성이 없으면 기본 각도와 선택 도구를 사용하고, 비유한 기울기는 무시하며 축별 ±90° 범위로 제한합니다. 펜 지우개/버튼 동작은 현재 브러시 모양·필압 설정을 유지하고 알파를 지웁니다. 획 도중 펜 방향/버튼 상태가 바뀌어도 지우개 여부는 누를 때의 값으로 끝냅니다.

버튼 옵션은 기본 꺼짐입니다. 입력 제공자가 펜 촉 자체를 버튼으로 포함할 수 있으므로 개별 버튼 번호를 임의로 가정하지 않고 보고된 버튼 중 하나가 눌린 상태에 적용합니다. 입력 설정에서 마지막으로 보고된 속성을 확인할 수 있으며 None은 해당 속성 미제공입니다. 버튼별 세부 배정은 별도 기능에 있습니다. 실제 펜 기울기·지우개·버튼 시험은 대기입니다.

Windows에서 펜 제공자가 연결되어 있으면 OS가 펜에서 만든 것으로 표시한 호환 마우스 입력은 문서 획에 다시 넣지 않습니다. 메뉴/버튼에는 기존 입력을 유지합니다. 실제 마우스는 펜과 같은 위치에 곧바로 눌러도 시간·거리로 제거하지 않습니다. 펜 제공자가 연결되지 않으면 호환 마우스를 기본 마우스 입력으로 사용할 수 있습니다. 입력 누락/큐 초과는 미완성 획을 버리고 새로 누를 때 다시 시작합니다. 장치가 출처를 표시하지 않은 이벤트를 임의로 추정해 제거하지 않습니다.

필압이 없는 마우스/펜은 압력 1로 그립니다. 기울기가 없으면 추가 회전 0을 사용합니다. 장치가 실제로 0압력을 보고한 경우와 속성이 없는 경우는 구분하며, 실제 0은 선택한 필압 연결 규칙대로 처리합니다. 속도나 이벤트 간격을 압력 누락의 대체값으로 추정하지 않습니다. 마우스 가상 필압/속도 반응은 별도 기능의 명시적 설정이며 기본 입력은 일정합니다.
