# 로컬 모델 형식과 사용조건 — 기능230

파일 선택 화면의 지원표는 `model_source::SUPPORT`와 동일한 어댑터 집합을 사용한다. 여섯 확장자를 모두 실제 가져오기와 합성 마우스로 검사한다. 개별 파일의 변환 손실은 기존 필수 검토/Apply/Cancel 흐름에 표시한다. 원본은 읽기 전용이며 네트워크·UNC·임의 외부 리소스 자동 추적은 없다.

| 형식 | 보존/변환 | 제외 또는 거절 |
|---|---|---|
| OBJ+MTL | polygon 삼각화, UV/법선, diffuse PNG/JPEG, 재질 색/알파, 원본/MTL 주석 | 표준 rig/morph 없음. 고급 MTL은 안내. 누락 참조는 적용 전 표시 |
| glTF2 / GLB2 | triangle/strip/fan, base color/alpha 맵, +Y/metre 세계 좌표, 최대128본/4가중치의 기본 LBS와 메타데이터, 위치 형태키32개를 자체 편집 키로 변환 | animation/PBR shading/모프 normal/tangent 제외 안내. sparse/미지원 필수 확장/누락 질감 거절 |
| VRM0.x /1.0 | glTF 변환 + 전체 VRM 확장 메타데이터의 값, 사용조건, humanoid/표정 정의 보관 | humanoid 리타게팅·MToon·스프링/constraint 동작 미구현을 안내. 알 수 없는 필수 확장 거절 |
| FBX ASCII/binary | 7.1–7.7(7100..7700) 고정 ufbx parser, polygon 삼각화/UV/법선, diffuse color/opacity/로컬 PNG/JPEG, 기본 선형/rigid 스킨, 단일 목표 위치 blend shape | 기본 포즈만; 애니메이션/고급 shader/PBR/opacity maps 제외 안내. DQ/복수 스킨/5이상 가중치/128초과 본/중간 blend shape/cache/points/lines 거절 |
| STL ASCII/binary LE | 삼각형 좌표, winding의 법선 재생성, solid label/80-byte header 출처 보관 | 표준 재질/알파/UV/리그/모프/사용조건 없음. 비표준 facet color 안내. count/길이 불일치·퇴화 면 거절 |

모든 어댑터는128개 mesh, 개별65536정점/간선·131072면·256재질과 문서/메모리 예약을 검사한다. PNG/JPEG8MiB·1024변 한도를 재사용한다. FBX/STL32MiB, 기존 OBJ8MiB·glTF/GLB16MiB와 리소스 한도를 적용한다. FBX parser는 외부 IO/캐시 평가를 끄고 임시/결과 allocator 제한 및 취소 callback을 사용한다. 텍스처는 상대 경로를 검증하고 모델 폴더 밖·UNC·URL 참조를 거절한다. FBX 별도 UV transform은 변환되지 않음을 안내하고 자체 슬롯 UV 조절을 제공한다. 스킨은 기본 포즈에 고정하고 joint parents/world/inverse bind/weights를 보관하며 독자 마네킹 rig로 가장하지 않는다.

사용조건: glTF asset.copyright·root/asset extras, VRM extensions/meta, FBX license/copyright/author/comment/usage/terms 노드, OBJ/MTL 주석·STL header/label을 원본 파일 경로와 필드 출처로 보관한다. 조건 문자열 값/줄바꿈은 유지하며 구조화 JSON의 공백은 정규화한다. 주석/헤더는 분류되지 않은 자료이며 허가로 해석하지 않는다. 자료가 없으면 정보 없음을 표시한다. UTF-8 실패/NUL·잘못된 VRM 조건 타입·최대64자료/개별64KiB/총256KiB 초과는 오류로 반환한다. VRM URL은 텍스트로만 보존하며 접근하지 않는다.

메타데이터와 변환된 장면/픽셀/스킨/형태키는 native121에 내장하고120 위장을 거절한다. 준비/취소·오래된 문서·한 Undo/Redo·원본 바이트 불변은 기존 model-library 작업자를 재사용한다. 형태키는 가져온 스킨의 기본 포즈 기준 세계 delta를 배치 시 자체 로컬 키로 변환하며 이후 weight0..1에서 메시를 재생성한다. 노멀 delta는 보존하지 않아 비영 키는 기하 법선을 사용한다.

공개 파서/형식 근거: [ufbx Rust 고정 원본](https://github.com/ufbx/ufbx-rust/tree/856c76840977cd504f053bf17ab4948b675b981a), [ufbx 변경 기록](https://github.com/ufbx/ufbx/blob/main/misc/changelog.md), [VRM1 meta 명세](https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_vrm-1.0/meta.md). 테스트 모델/작성기는 자체 수치 자료이며 공개 예제 모델은 동봉하지 않는다.

주석/헤더 자료가 있어도 사용조건 필드/표식이 없으면 `정보 없음`을 함께 표시한다. FBX 유색 투명도는 휘도 알파로 변환한다고 안내한다. 원본 좌표축 반사에 따른 면 winding도 함께 변환한다.

Windows 배포를 위해 고정 ufbx 소스를 vendor/ufbx에 보관한다. 파서/바인딩은 그대로이며 빌드 설정만 MSVC /MT·/Zl로 바꿔 C 함수의 직접 호출을 사용하고 최종 Rust 바이너리가 CRT를 선택하게 한다. 동적 테스트와 정적 배포 모두 같은 C 객체를 사용한다. 원본 crate 체크섬·리비전·모든 로컬 소스 해시는 patched-sources.json/고지에 기록한다.

배포 전 PE import/delay-import 표에서 VC 재배포/디버그 CRT DLL을 거절하며 개발도구 PATH 없는 설치본 실행도 수행한다. 전달 목록의 C/header 예외는 고정 vendor/ufbx/ufbx 경로에 한정한다.

고정 의존 소스는 .gitattributes에서 줄바꿈 자동 변환을 제외한다. 커밋된 각 파일의 원시 바이트까지 patched-sources.json의 해시와 대조하며 이 설정도 전달 묶음에 포함한다.
