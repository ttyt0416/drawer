# 실치수 보기와 선택 인쇄 (256)

기존 Print preview 창의 Content / channels / RGB ICC / physical screen calibration을 연다. 화면의300 논리 포인트 선에서 양 끝 눈금 중심 사이를 자로 측정하고 mm를 입력하면 현재 화면의 물리 픽셀/mm를 계산한다. 현재 보기의 DPI/회전/반전/배율과 분리한 보정값을 인쇄 설정에 저장한다. Show active page at physical print size는 현재 문서 DPI로 보기 확대율만 계산하고 보기 중심을 유지한다. 그림 픽셀·DPI·이력에는 넣지 않는다. 다른 모니터를 사용할 때는 다시 측정한다. 지원하는 보기 확대율0.01..64를 벗어나면 적용하지 않는다.

활성 페이지 인쇄는 편집 가능한 읽기 전용 사본을 유지한다. 밑그림·이 페이지의 문자/번호·설정된 재단/도련/안전 가이드를 선택하면 같은 사본에서 다시 합성한다. 원문서의 출력 플래그를 바꾸지 않는다. 이미 합성된 래스터 페이지에는 분리할 레이어가 없으므로 이 내용 스위치를 비활성화하고 그 이유를 표시한다. 등록 십자는 종이 좌표에 별도로 포함/제외하며 inset을3..100mm로 정한다. 가상 PDF·화면·실제 인쇄의 같은 위치를 사용하고 드라이버가 그 위치에 닿지 못하면 작업 제출 전에 거절한다.

합성 RGB 또는 R/G/B의 중성 회색 분판·알파 커버리지(불투명=검정)를 선택한다. RGB 분판은 원래 알파를 유지해 투명 영역이 종이 위에서 비게 한다. 인쇄 페이지 입력은 기존처럼 합성 sRGB8이다. RGB ICC와 Perceptual/Relative/Saturation/Absolute 의도를 선택하며 Windows ICM을 끄면 값은 보존하고 수동 ICC를 비활성화한다. 모니터 ICC와 인쇄용 ICC는 독립이다.

수동 출력 RGB ICC는 로컬 경로/재분석 지점·4MiB·RGB 구조를 검사한다. 미리보기는 기존 검증된 RGB 변환기의 대상 변환→8비트 양자화→sRGB 표시로 출력 범위를 모사한다. 따라서 가상 PDF는 그 RGB 모사 결과를 포함한다. 실제 인쇄는 분판한 sRGB 원본을 전달하고 Windows ICM의 선택 출력 프로파일과 V5 의도로 변환한다. CMYK 출력 ICC로 가장하지 않으며 실제 프린터/종이의 길이·색 측정은 별도다.

검토한 ICC 바이트 해시를 비교하고 읽기 전용 임시 사본을 작업 끝까지 유지한다. 원래 프로파일이 검토 후 바뀌면 새 검토를 요구하고 출력하지 않는다. 임시 사본은 동기화·배타적 생성·Windows 읽기 공유로 보호하고 완료/취소 뒤 정리한다. 인쇄 창을 닫아도 취소를 요청한 작업 결과를 받기 전까지 소유권을 유지한다. 그림 문서를 닫아도 이미 캡처한 인쇄 사본은 독립이며 빈 문서를 만들지 않는다.

Microsoft의 [SetICMProfileW](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-seticmprofilew)로 해당 DC의 출력 ICC만 지정한다. 시스템 전체 장치 연결은 바꾸지 않는다. [BITMAPV5HEADER](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-bitmapv5header)의 의도1/2/4/8과 명시적 sRGB를 사용하고, [StretchDIBits](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/nf-wingdi-stretchdibits)는 활성화한 ICM에서 변환한다. 외부 프로파일 링크를 DIB에 기록하지 않는다.

검증:100mm 문서의 실제 보기 길이 계산을 배율1/1.5/2와 회전/반전에서 비교한다. 잘못된/최소·최대 보정값, R/G/B/A 값·알파·원본 불변, 밑그림/문자/재단 가이드 포함·제외, ICC 의도/해시 동결/임시 정리·손상/누락/비활성 옵션, 등록 표시의 가상 출력·메모리/취소·설정 재시작을 검사한다. 실제 입력으로 내용 재합성·채널/등록 표시·측정 보정/실치수 보기·저장·검토 뒤 ICC 변경 거절을 확인한다. 인쇄 구조 테스트는 V5 크기/방향/색공간/의도를 대조하며 실제 프린터 작업을 자동 제출하지 않는다.
