# 설치 자원과 실행 요구 파일

| 기능 | 공급 위치 | 누락 시 처리 |
| --- | --- | --- |
| 기본7브러시·커서·아이콘·기본3D도형·절차 질감 | Drawer.exe 안의 독립 Rust 수식/픽셀 배열 | 외부 자산 불필요, 설치본 기능 시험으로 확인 |
| 기본 대체 글꼴4종 | egui/epaint 기본 글꼴, 실행 파일에 정적 포함 | 외부 글꼴 폴더 불필요 |
| 한글 UI 보조 글꼴 | Windows Fonts/malgun.ttf, 선택적 읽기 | 내장 글꼴 유지, 상태 표시; 다운로드 없음 |
| 도움말·고지 | 실행 파일 include_str! + HELP.md, THIRD-PARTY-NOTICES.txt | 앱 안에서 로컬 열기; 배포 검사에서 누락 거절 |
| 이미지·PSD·모델 파서·추론 엔진 | 잠긴 Rust/C 의존성 정적 연결 | 별도 언어 런타임/VC 재배포 DLL 불필요 |
| Windows 창·GPU·입력·음성/인쇄 | 지원 Windows의 시스템 DLL/드라이버 | 운영체제 장치 지원은 해당 기능의 상태로 표시 |
| 사용자 동영상 디코더·추론 모델 | 사용자가 별도 선택한 로컬 파일 | 선택 기능만 실행 거절, 기본 그리기/저장 유지 |
| 사용자 프리셋·소재·설정·검색/복구 | 설치 경로 밖 UserStorage/Scratch | 사용자가 선택한 로컬 저장소에 기록 |

배포12파일: Drawer.exe, THIRD-PARTY-NOTICES.txt, COMPONENTS.json, Cargo.lock,
OWN-ASSETS.md, START-HERE.md, MOVIE-IMPORT.md, IMAGE-IMPORT.md, HELP.md,
MODEL-FORMATS.md, Install.ps1, Uninstall.ps1. 설치 소유 기록1개가 추가된다.
package.ps1은 이 목록과 링크를 검증하고 verify-pe.py는 일반/지연 import의
VC 재배포/디버그 CRT DLL을 거절한다. COMPONENTS에는312개 라이브러리와4개 글꼴의 고지가 있다.

test-installed-assets.ps1은 실제 Windows ACL 쓰기 거부 대조군을 확인하고
읽기전용 설치 디렉터리를 작업 위치로 전체 내장 기능 시험을 실행한다.
설치 전후13파일의 SHA256/목록 동일, 별도 사용자 설정/소재 저장을 검사한다.
결과는 installed-assets-evidence.json이며272의 새 오프라인 PC 시험을 대신하지 않는다.

277부터 PACKAGE.json이 추가되어 현재 배포13파일/설치 기록 포함14파일이다. 274의 evidence는 당시12/13파일 배포 해시의 기록으로 보존한다.
