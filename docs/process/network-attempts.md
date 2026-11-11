# 통신 시도 검사

test-network-attempts.py는 새 합성 저장소에서 배포 앱 전체 시험과 별도 Windows
기본 PowerShell 수동 설치를 실행한다. win-api-probe.py는 이때 직접 시작한
프로세스/자식만 Windows 디버그 객체로 관측하고 기존 앱에 붙지 않는다.
DNS·HTTP/WinHTTP·WinInet·주소 해석·소켓/연결·bind/listen/accept·URL 다운로드·
CryptNet URL 검색·WerReportSubmit의 명시한 사용자 모드 진입점을 검사한다.

시도를 관측하면 원래 API 명령을 실행하기 전에 정지된 모든 자체 프로세스를
종료한다. 인터넷 부재/잘못된 인자 때문에 실패할 호출도 시도로 세고 실패한다.
앱/설치 시나리오의 완료 기준은 시도0·루트 정상 종료·잔여 프로세스0이다.
인자/그림/실제 사용자 경로를 수집하지 않는다. 대조군10개는 소켓, 연결, 바인드,
듣기, 주소 해석, DNS, WinHTTP, WinInet, URL 다운로드, 오류 제출을 각각 호출한다.
루프백 서버도 bind/listen 진입 전 중지한다. 대조군의 원래 호출은 실행하지 않는다.

앱은 Rust main 시작 시 GetErrorMode의 기존 비트를 유지하며 SetErrorMode로
SEM_NOGPFAULTERRORBOX를 설정해 해당 프로세스의 Windows 오류 보고 호출을 억제한다.
helper도 같은 진입점을 거친다. 오류 UI 억제도 프로세스 범위이며 WER 제외
레지스트리/호스트 정책을 수정하지 않는다. 실제 비트를 설치본 영수증으로 검사한다.

network-attempts-evidence.json은 이 유한 시나리오·지정 API의 실제 관측이다.
직접 시스템 호출/커널 SMB/별도 Windows·드라이버 서비스/앱 main 이전의 장애를
포괄하지 않는다. 호스트 네트워크 장치/방화벽을 변경하지 않는다. API 수준으로
차단한 실행과 실제 새 PC 네트워크 차단 시험272는 구분한다.272는 별도 미완료다.
