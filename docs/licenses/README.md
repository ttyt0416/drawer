# 구성요소·출처·배포 고지

`inventory.json`은 잠긴 Windows x64 의존성 그래프의 라이브러리 버전·배포 출처·선언 라이선스·아카이브 체크섬·고지 파일 체크섬을 기록한다. 내장 글꼴 네 개와 직접 제작한 아이콘/팁의 제작 기록도 연결한다. 외부 모델은 사용하지 않는다.

`third-party-notices.txt`에는 의존성에 포함된 고지문과 정확한 공개 소스 리비전에서 보충한 고지문을 보존한다. 기본 글꼴의 Hack·Noto Emoji·Ubuntu·emoji-icon 고지문도 포함한다. 보충 파일의 URL·리비전·체크섬은 `upstream-sources.json`에 있다.

`python scripts/license-inventory.py --check`는 잠금 파일과 현재 그래프를 다시 계산하여 기록 및 고지문이 같은지 확인한다. 누락·변경 시 배포 묶음 생성을 중단한다. 의존성을 바꾸면 목록을 다시 생성하고 출처를 검토한다.

`scripts/package.ps1`은 배포 폴더에 실행 파일·고지문·구성요소 목록·잠금 파일·자체 소재 기록을 함께 넣는다. 런타임 다운로드나 계정 인증은 사용하지 않는다.

영상 가져오기는 사용자가 이미 설치한 FFmpeg/FFprobe 7.1을 선택적 외부 프로세스로 사용한다. 이 저장소/앱/설치/인계 묶음에 코덱 실행 파일·DLL을 배포하지 않는다. 실제 검증 환경은 `7.1-essentials_build-www.gyan.dev`이며 해당 도구의 `--enable-gpl --enable-version3` 빌드 구성을 확인했다. 코덱의 설치·고지/배포 의무는 그 도구 배포에 속하며 앱은 설치나 자동 다운로드를 수행하지 않는다. 허용 입력·출력과 검증 기록은 `docs/process/movie-import.md`에 있다.
