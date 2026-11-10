# 로컬 PDF·PSD/PSB 출력 옵션 (257)

PDF/image EPUB 창의 PDF 옵션을 펼친다. 입력 페이지 범위와1..4800DPI는 기존대로 실제 문서 픽셀 수를 결정한다. Left/Right binding은 가져온 각 페이지의 제본 메타데이터다. From left/right는 선택 페이지의 오름/내림 순서이며 파일의 좌우 픽셀을 뒤집지 않는다. 가져온 문서는 독립 래스터 문서이고 원본 PDF는 변경하지 않는다.

Import annotations as image는 선택한 페이지의 정적 AP/N 외형(AS로 선택된 외형 포함)을 일반 페이지 Form XObject로 합성한 뒤 Windows PDF 렌더러로 이미지화한다. Rect와 BBox/Matrix를 사용해 같은 위치에 놓으며 숨김/비표시 주석은 제외한다. 끄면 모든 주석을 제거한 스냅샷을 렌더링한다. 주석 자체·댓글 편집은 가져오지 않는다. 정적 외형이 없거나 미검증 불투명도/혼합/선택적 표시를 가진 주석은 켜기에서 거절하고 원앱의 평면 PDF 내보내기를 안내한다. 원격 URI/실행/스크립트/외부 스트림/첨부/암호화/양식은 옵션과 관계없이 렌더러 호출 전에 기존 정적 검증에서 거절한다.

출력 ZIP은 RGB/알파 무손실이다. JPEG Highest/High/Medium/Low/Lowest는 각각 품질100/90/75/50/25, RGB4:4:4 DCT이며 알파 SMask는 항상 무손실이다. 원문서의 정확한 픽셀은 JPEG와 별개로 유지된다. PDF1.5에 단면/연속단면/맞쪽/연속맞쪽과 L2R/R2L 기본 보기 방향을 기록한다. 맞쪽의 홀수 페이지 위치는 제본 방향으로 선택하며 출력 페이지 순서는 선택 순서 그대로다. 뷰어가 이 초기 보기 힌트를 무시할 수 있다. 제목/저자/언어는 Unicode로 기록한다. 로컬 PDF 옵션 저장은 그림 이력과 분리되며 손상 설정은 확인한 교체만 허용한다.

PSD/PSB 검토에는 텍스트·말풍선의 레이어/의존 범위별 래스터 대체와 편집 손실을 표시한다. 검증된 편집 텍스트 어댑터로 가장하지 않는다. Include real merged compatibility preview를 끄면 레이어 사본은 그대로이고 Version Info1057의 hasRealMergedData=false와 필수 Image Data 자리의 투명 placeholder를 쓴다. 따라서 합성본이 필요한 다른 프로그램에서는 거절할 수 있다. Drawer는 검증 가능한 레이어만 다시 합성하고 그 사실을 보고한다. 미검증 레이어와 실제 합성본이 없는 입력은 빈 합성본으로 대체하지 않고 거절한다. 자체 .drawer의 텍스트·말풍선 편집 상태와 그림 실행 취소는 바꾸지 않는다.

형식 근거: [Adobe Photoshop 공개 규격](https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/)의 Image Data와 Version Info1057, [ISO32000-1 공개본](https://opensource.adobe.com/dc-acrobat-sdk-docs/standards/pdfstandards/pdf/PDF32000_2008.pdf)의 PageLayout/ViewerPreferences 및 annotation appearance 규칙을 따른다. PDF 보기 설정과 실제 페이지 픽셀의 순서는 독립이다.

검증은 여섯 압축×네 배치×두 방향의 구조/한글 메타데이터/알파/무손실 픽셀·다섯 JPEG 페이로드, 주석 제거/정적 합성/취소/원격 참조 거절, Windows 렌더러의 주석 유무 픽셀과 역순 선택을 대조한다. 텍스트·말풍선 사본을 PSD/PSB와 합성본 포함/제외 조합으로 다시 열어 같은 외형과 손실 보고·Native 편집성을 확인한다. 실제 포인터로 옵션·압축/배치·저장·PSD 미리보기 검토 무효화와 독립 가져오기/출력 작업을 검사한다.
