# 여러 팁과 혼색·시작끝 방식

기능065. 자체 마스크 참조0(현재)/1~7을 최대8슬롯에 저장한다. 순서·시드·참조는 도구 입력/프리셋/가상 기록/문서v17의 획 스타일에 보존한다. 구형 누락은 단일 기본 팁이다. 알 수 없는 번호는 지우지 않고 경고와 현재 팁 대체를 적용한다. 외부/클라우드 소재를 읽지 않는다. 임의 생성기는 명시적 u64 wrapping 연산과 시드로 고정하며 한 임의 주기는 Fisher–Yates 순열을 한 번 소비한다.

Repeat 순환, Reverse 끝 중복 없는 왕복, Do not repeat 마지막 유지, Random 복원추출, One time only 전진 후 중단, One random cycle 비복원 한 주기 후 중단이다. 1슬롯과 종료 후 호출도 명시적으로 처리한다. 이미지 마스크만 교체하고 원래 도구의 지우개/마커/번짐 역할은 바꾸지 않는다. 실제 도장 마스크를 커서에도 사용한다. 기존 팁 모양 Round/Square 강제 설정은 마스크 외곽에 우선한다. 여러 팁 문서 획은 원본에서 결정적으로 재계산하고 결과 픽셀/스타일을 한 이력으로 게시한다.

Running color는 현재 도장의 층 색과 공급된 주색을 mixing으로 혼합하며 과거 운반색을 유지하지 않는다. Blend/Smear/Water는 기존 운반 모델이다. Perceptual은 Oklab 좌표에서 알파 가중 혼색을 하고 최종 표시/레이어 합성은 기존 sRGB다. High quality는 Running color의 회전된 팁 내부9점 평균으로, 선택/캔버스 밖은 제외하고 유효 선택 coverage로 가중한다. Normal은 중심 샘플이다. 픽셀 작업량 예산에 샘플 작업을 포함한다. 원본 앱과 내부 수치 호환은 주장하지 않는다.

시작끝 Specify length는 px, By percentage는 전체 실제 경로 길이×백분율이다. Fade는 시작 크기1에서 지정 거리까지 최소 배율로 줄인 후 유지한다. 점 클릭은 기존 계수1이다. .5px 재표본화와 시작끝 절점으로 빠른 이벤트 두 개만으로도 중간 기하를 보존한다. Scratchpad는 독립 원본 문서와 한 pending Stroke를 유지해 이벤트마다 순서/운반 상태가 재시작되지 않는다. 정상 Up에 게시, Esc/포커스/팝업 종료/오류에는 전체 미리보기를 복원한다.

공개 기능 의미 참고: [CLIP STUDIO PAINT S 옵션](https://help.clip-studio.com/en-us/manual_en/810_subtools/S.htm), [I 옵션](https://help.clip-studio.com/en-us/manual_en/810_subtools/I.htm). 공개 설명의 의미만 참고해 새 Rust 상태기계와 혼색/재표본화 알고리즘을 작성했다. 원본 코드/소재를 사용하지 않았다.

지각 좌표 참고: [Björn Ottosson의 Oklab 정의](https://bottosson.github.io/posts/oklab/). 공개 도메인 기준 행렬의 수학을 독립 Rust f64 식으로 표현했다. 기준 빨강 좌표/정방향·역방향/투명 RGB 무영향을 검증한다. 추가 라이브러리나 원본 앱 구현을 도입하지 않았다. 물리 펜 시험은 대기다.
