# 회색조·알파 채널과 해상도 메타데이터

251: PNG RGB/RGBA 및 Gray/GrayAlpha8·16비트를 구분해 실제 채널 수와16비트 큰엔디언 샘플을 기록한다. 회색조는 sRGB를 선형화한 상대 휘도0.2126R+0.7152G+0.0722B를 sRGB 전달 곡선으로 다시 부호화한다. 불투명 출력은 사용자가 고른 배경과 합성한 뒤 회색조로 변환한다. PNG는1/2채널, JPEG 등 나머지 어댑터는 동일값3채널을 사용하며 JPEG는 실제 압축 해독을 검토한다. 워터마크와 노이즈도 최종 채널 변환을 거친다.

PNG Gray ICC는 자체 sRGB 전달 곡선·표준 백색의 GRAY 프로파일로 만들고 결정성 있는 시각을 기록한다. ICC 제외 시 sRGB 청크로 전달 함수를 표시한다. 자체 RGB/Gray 프로파일을 정확히 식별하여 자체 PNG를 다시 읽을 때 양자화된 ICC 행렬/곡선 때문에 저비트가 반복 변환되지 않게 한다. 외부 Gray 프로파일은 기존 색 관리 어댑터로 변환하며8/16비트 알파는 별도로 보존한다. 자체 문서의 정밀도·프로파일·원래 색상은 변경하지 않는다.

BMP24는40바이트 BITMAPINFOHEADER/BI_RGB, 아래에서 위 순서의 BGR와4바이트 행 정렬이다. BMP32 알파는124바이트 BITMAPV5HEADER/BI_BITFIELDS, 명시적인 RGB/alpha 마스크와 비연관 BGRA, sRGB·외부 프로파일 참조 없는 헤더다. 자체 BMP 쓰기와 별개인 image 디코더로 알파/행 순서를 검사한다. 회색조와 BMP는 출력 sRGB를 강제하므로 RGB ICC 대상·변환·의도 수동 선택은 비활성화하고 값을 보존한다. 프로파일 포함 여부는 Gray PNG/다른 RGB 회색조 지원 형식에서 적용한다.

PNG pHYs와 BMP X/Y pixels-per-meter는 DPI/0.0254를 정수로 반올림한다.1inch=0.0254m이며 반올림 후 DPI는 ppm*0.0254로 표시한다. JPEG JFIF는 정수 DPI, TIFF는 기존1/10000인치 해상도를 사용한다.72.5/299.75DPI에서 정수 필드 근사 안내를 검증하며 원문서 DPI에 근사값을 역대입하지 않는다. 실패/취소 시 기존 출력은 기존 원자적 쓰기로 보존한다.

검증은 색/회색조×알파 켬/끔×8/16비트×ICC 포함/제외, PNG 헤더/원시 채널/재가져오기/저비트·알파, BMP INFO/V5/마스크/독립 디코더/행 패딩·잘못된 헤더·잘린 파일, RGB 동일값 JPEG와 불투명 배경, 비활성 RGB ICC 경로와 옵션 값 보존, 실제 창의 형식 왕복/회색조 포인터·GrayAlpha16/BMP32 출력/취소 및 원문서 불변을 포함한다.

공개 형식 근거: [W3C PNG](https://www.w3.org/TR/png/), [Microsoft BITMAPINFOHEADER](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-bitmapinfoheader), [Microsoft BITMAPV5HEADER](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-bitmapv5header). 새 앱의 독립 구현이며 원제품 코드/리소스를 포함하지 않는다.
