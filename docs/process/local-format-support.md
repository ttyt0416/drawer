# 로컬 문서·도구 호환 범위 (258)

| 원앱 | 문서 종류/알려진 확장자 | 도구 파일 | Drawer 직접 호환 |
|---|---|---|---|
| IllustStudio | 문서 .xpg | .tos 도구 세트 | 각각 미검증·미지원 |
| ComicStudio / Manga Studio | 페이지 .cpg, 작품 .cst, 스토리보드 .csnf | .tos 도구 세트 | 각각 미검증·미지원 |
| ibisPaint | 작품 .ipv | 독립 도구의 확장자/어댑터 미검증 | 각각 미검증·미지원 |
| kakooyo! | 작품 .kako | 독립 도구의 확장자/어댑터 미검증 | 각각 미검증·미지원 |
| CLIP STUDIO PAINT | .clip / .lip | .sut 하위 도구 | 각각 미검증·미지원 |
| SAI / SAI2 | .sai / .sai2 | 전용 도구·설정 어댑터 미검증 | 직접 문서·도구 어댑터 없음 |

이 표는 원앱의 파일 유형 식별과 Drawer의 지원 여부를 구분한다. 다른 제품에서 열 수 있다는 사실은 Drawer의 호환성을 뜻하지 않는다. 문서는 원앱에서 PNG/PSD로 별도 내보낸 뒤 확인해 가져오며 편집 원본은 보존한다. 도구는 자체 JSON/portable 묶음 또는 별도 검증된 ABR 가져오기만 사용한다. 일반 PNG로 저장된 브러시 QR 이미지를 그림으로 여는 것은 도구 설정 가져오기가 아니다.

기본 파일 선택 창의 호환 범위와 사용자 도구의 휴대용 교환 영역에서 같은 표를 확인한다. 알려진 미지원 확장자는 실제 픽셀/JSON이 들어 있어도 전용 문서/도구로 읽거나 쓰지 않는다. 기존 파일 선택·최근·이미지/층 가져오기·드롭과 도구/브러시/색상표/액션 읽기·쓰기에 같은 안내를 사용한다. SQLite 헤더는 컨테이너임만 식별한다. 앱 이름이나 레이어를 추정하지 않으며 .png/.tga/.drawer/.json 등으로 이름을 바꾸어도 읽지 않는다. 알려지지 않은 이진 데이터는 검증된 스키마/이미지 디코더를 통과하지 못하면 실패한다. 일부 구조가 읽힌 것만으로 성공 표시하지 않는다.

확장자 근거: CELSYS의 [페이지 교체 설명](https://www.clip-studio.com/site/gd_en/csp/userguide/csp_userguide/500_menu/500_menu_page_replace.htm)은 xpg/cpg/csnf/ipv/kako를 구분한다. [공식 ComicStudio 지원 문서](https://support.clip-studio.com/en-us/faq/articles/20190053)는 cpg 페이지와 cst 작품을 구분한다. [공식 브러시 형식표](https://support.clip-studio.com/en-us/faq/articles/20200233)는 tos와 sut를 구분하며, [ibisPaint 파일 이동 설명](https://ibispaint.com/lecture/index.jsp?no=205)은 작품 IPV를 설명한다. 이 이름만으로 비공개 파일 내부의 호환 구현을 주장하지 않는다.

검증은 모든 알려진 미지원 확장자의 내용 위장·자체 JSON 도구 위장·읽기/쓰기 거절과 원본 바이트 보존을 검사한다. SQLite 컨테이너의 이름 위장과 원격 참조/비공개 부분 읽기 경로 부재를 확인한다. 실제 파일 선택 콜백·이미지·새 층·egui 드롭이 같은 안내를 내며 기존 문서 상태/실행 취소/열린 문서 수를 보존하는 기존 회귀 검사를 확장했다.
