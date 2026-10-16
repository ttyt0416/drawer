# CIELAB D65 혼색·밝기와 두 잉크 브러시

기능079. Tool input의 안료 설정은 Standard 문서 RGB, Perceptual CIELAB D65, 기존 Perceptual Oklab을 구분한다. 저장된 Perceptual 값의 Oklab 결과는 유지한다. 내부 작업 픽셀은 기존 sRGB RGBA8이며 CIELAB은 sRGB 전이함수 해제→D65 XYZ→CIE1976 Lab→알파 가중 보간→역변환→sRGB8 순서다. D65 백색은 x=.3127/y=.3290에서 도출하고 색역 밖은 RGB 범위로 제한한다. 불투명 검정/흰색의 절반은 Lab에서119회색, 문서 RGB에서128회색이다. 투명색의 숨은 RGB는 보간에 기여하지 않고 비율0/1은 정확한 입력 끝값이다.

Lightness correction toward input L*는0–1 강도다. 보간 후 결과를 D65 Lab으로 바꿔 그 L*를 알파 가중 입력 L*의 평균으로 이동하고 a*/b*/알파를 유지한 뒤 RGB로 돌아간다.0은 정확히 기존 결과이며 색역 제한/8비트 반올림은 남는다. 이 제어는 안료 pickup/혼색/운반색 보간에 적용하며 렌더링 품질과 별개다.

공개 정의의 수학만 독립 구현했다. CIELAB 식은 [CIE 용어 정의](https://cie.co.at/eilv/157), sRGB/D65 행렬·전이함수는 [W3C 색 변환 정의](https://www.w3.org/TR/css-color-4/#color-conversion-code), 표준 분리 합성은 [W3C 합성 정의](https://www.w3.org/TR/compositing-1/#blendingseparable)를 확인했다. CSS의 D50 Lab 선택 대신 이 프로그램 명세의 D65 백색을 사용한다. 원본 프로그램/표준의 구현 코드를 복사하지 않았다.

보조 팁은 기존 Round/Square/Grain/Soft 자체 마스크에 별도 RGBA를 가진다. Link secondary size가 켜지면 현재 주 팁 지름의0.1–1 비율이고, 끄면0.1–4096문서px의 독립 지름이다. 회전과 주 팁 반지름 기준 오프셋은 기존과 같다. 동역학으로 바뀐 주 지름에도 연동 여부를 적용하고 독립 지름의 외곽을 포함해 경계/작업량을 제한한다. 크기0으로 소스를 만들지 않는다.

필수14모드 Normal, Multiply, Add (Glow), Subtract, Darken, Lighten, Screen, Overlay, Color dodge, Color burn, Linear burn, Hard mix, Height (Linear), Apply RGB value와 기존 Difference를 제공한다. Darken/Lighten은 기존 Minimum/Maximum 직렬화 이름을 유지한다. 두 정규화 값 a,b에 대해 Multiply=a×b, Add=min(1,a+b), Subtract=max(0,a-b), Darken=min, Lighten=max, Screen=a+b-a×b, Overlay는 a≤.5에서2ab/그 외1-2(1-a)(1-b)이다. Dodge/Burn은 표준의0/1 경계 우선순위를 사용한다. Linear burn=max(0,a+b-1), Hard mix는 a+b≥1에서1/그 외0, Height (Linear)는(a+b)/2다. 추가 모드의 수학은 자체 규칙이며 다른 제품의 같은 이름 결과와 일치한다고 주장하지 않는다.

알파 전용은 주 잉크를 유지하며 보조 알파×보조 mask를 합성한다. Normal의 coverage는 source-over 합집합, Apply RGB value의 coverage는 주 coverage×보조 coverage×보조 휘도다. 나머지는 위 스칼라 규칙이다. 휘도는 (54R+183G+19B)/(256×255)다. 합성 coverage와 원래 coverage 사이를 보조 opacity로 보간한다. 주 잉크 알파는 기존처럼 마지막 색 공급에 포함하므로 투명 주색에서 알파 전용 마스크는 색을 만들지 않는다.

Combine secondary RGB and alpha는 안료 처리 후의 주 잉크와 독립 보조 잉크를 합친다. 각 잉크의 coverage×알파를 먼저 계산해 같은 모드로 최종 coverage를 만들고, RGB는 보조를 source로 하는 알파 가중 source-over와 채널 합성으로 구한다. 결과 알파를 coverage에 포함하므로 다시 잉크 알파를 곱하지 않는다. 보조 opacity는 각 알파/색 기여에 한 번 적용한다. 두 잉크가 완전투명하면 색이 없고, 주 잉크만 투명하면 숨은 RGB가 보조색에 섞이지 않는다. Normal에서 빨강α128+파랑α64는 중심[153,0,102,160]이다. Normal/Screen은 coverage가 같아도 RGB 결과가 다르다. Apply RGB value는 보조 휘도가 색의 기여율도 제한한다.

지우개/복제/픽셀 덮어쓰기는 자기 RGBA 쓰기 규칙을 유지하고 보조 coverage만 사용한다. 알파 잠금/선택/문서·배치 레이어 경계/전체 농도/질감은 기존 공통 최종 경로를 쓴다. 획/도장별 HSV 변동은 기존 주 잉크의 시드·범위를 유지하고 보조 색은 명시한 RGBA다. 모든 색 변화와 두 팁은 한 그림 이력이며 설정 변경/Redo는 과거 픽셀을 다시 계산하지 않는다.

brightness 누락은0, dual.controls 누락은 RGB끔/불투명 검정/주 지름 연동/독립 지름8 기본값이다. 기존6종 마스크 수치와 픽셀을 회귀 검사한다. 프리셋/사용자 슬롯/전체 설정과 문서v28에 저장하고 재시작/로컬 묶음에서 보존한다.

시험: CIE 원색/백색/검정 기준과 RGB 격자 왕복·Lab119회색·밝기0/중간/1·투명 입력/비율0/1/잘못된 값·모드별 중간 수치/흑백 네 경계·대조색/두 알파/RGB on-off의 실제 픽셀·투명 주색·독립/연동 지름의 실제 폭·단일 Undo/Redo·문서 재열기·생산 입력15모드/투명 주색/Standard-Lab-밝기·프리셋/설정 재시작/그림 불변을 포함한다. 기존 HSV 획/도장 범위와 시드 재현 시험도 함께 실행한다. 실제 펜 하드웨어 검증은 대기다.
