# 로컬 OBJ·MTL·glTF2·GLB 가져오기 (221)

`model-library`의 Model file에서 `.obj`, `.gltf`, `.glb`를 고르고 Preview local model을 누른다. 기존3D 창에서 경고/손실 목록과 렌더를 확인한 뒤 검토 확인·Apply한다. 전체 자산이 한 Undo/Redo이고 준비/폐기/취소는 원본과 문서를 유지한다. 계산 중 문서 변경·문서 닫기와 작업 취소는 결과를 버린다. 가져오기와 복제는 외부 모델 파일에 쓰지 않는다.

OBJ의 다각형/라인·UV·법선·재질은 기존 판독기를 사용하고 MTL의 diffuse 색/불투명도와 `map_Kd` PNG/JPEG를 문서에 내장한다. 없는 MTL/텍스처는 목록에 남고 diffuse 대체 색을 사용한다. MTL 고급 명령/맵 옵션은 적용하지 않고 안내한다. OBJ의 축/단위는 자체 정의가 없으므로 회전/Scale로 지정한다. 내부 하위 폴더에서 모델 폴더 안의 참조로 정규화하는 기존 MTL 상대 경로를 허용하고 모델 폴더 밖 참조는 거절한다.

glTF2는 JSON/GLB2 JSON+BIN 컨테이너, 로컬 외부 버퍼/이미지·제한된 base64 data URI·이미지 bufferView를 읽는다. +Y 위의 오른손 좌표/미터를 유지하며 정점 노드의 누적 TRS/정규화 quaternion 또는 affine column-major MAT4를 적용한다. 행렬/TRS 혼용·순환·여러 부모·잘못된 장면 루트를 거절한다. 기본 장면만 선택하며 장면이 없으면 부모 없는 노드를 가져온다는 안내를 표시한다. 가져오기는 전체 자산을 중심에 배치하므로 원본의 절대 배치는 유지되지 않는다. Fit을 끄면 상대 길이를 유지하며 Scale은 단위 변환, 회전은 축 변환이다. Fit을 켜면 가장 긴 변을 지정 길이에 맞춘다.

지원된 메시 속성은 float POSITION/NORMAL, float 또는 normalized unsigned TEXCOORD_0와 unsigned8/16/32 인덱스이다. 정규화·수·버퍼/뷰/접근자 범위와 stride/정렬을 검사하고 TRIANGLES/STRIP/FAN을 삼각형으로 변환한다. 퇴화 삼각형은 개수를 안내하고 제외하며 반사 노드의 winding을 보정한다. sparse 접근자와 압축/양자화 확장에 의존하는 메시, point/line primitive, base color의 다른 UV 세트는 이유와 함께 거절한다. 알 수 없는 속성/확장은 경로/이름으로 나열하고 필수 확장은 가져오기를 거절한다. 선택하지 않은 데이터와 임의 application extras, 애니메이션/모프/추가 UV·정점색·추가 가중치는 원본 호환성을 보장하지 않는다.

재질은 선형 baseColorFactor를 내부 sRGB diffuse 색으로 변환하며 PNG/JPEG baseColorTexture의 RGB를 선형광에서 곱한다. UV는 편집기의 아래 원점 규약으로 바꾸고 텍스처는 위 원점 이미지로 표본화한다. 재질별 nearest 표본·repeat/clamp/mirror와 OPAQUE/MASK/BLEND coverage를 적용한다. MASK의 빈 픽셀은 깊이·가림·그림자 ray에서 제외한다. BLEND는 기존 개체별 최근접 면과 레이어 순서를 사용하므로 한 개체 내부의 겹치는 반투명 면 전체 합성은 지원하지 않고 안내한다. two-sided diffuse 렌더이며 금속/거칠기 BRDF·normal/occlusion/emissive 맵·필터링·원본 카메라/조명은 재현하지 않는다. 기존 lit 스타일은 유지하고 조명 없음/Wire는 같은 Apply에서 Unlit으로 바꿔 이미지를 표시한다. 후속 UV 채색은 내장 원본 맵과 alpha를 유지하면서 모델의 diffuse RGB 표본을 채색 맵으로 교체한다고 경고한다.

스킨은 JOINTS_0/WEIGHTS_0(최대4 영향), 관절 이름/조상 관계·원본 위치·관절 world/rest 행렬·inverse-bind 행렬을 내장한다. 기본 linear blend skinning 포즈를 메시/법선에 반영한다. 관절 인덱스·개수·가중치/벡터 수·유효한 역행렬을 검사하고 가중치를 합1로 정규화한다. 가져온 리그의 포즈는 정적 형상이며 내장 mannequin 포즈 창으로 재편집하지 않는다. 애니메이션/모프/추가 영향 손실은 적용 전에 안내한다. 리그 메타데이터와 텍스처는 native113으로 저장되며112로 위장한 파일은 거절한다. 원본과 독립적인 복제/저장·재열기를 제공한다.

URL/드라이브·절대경로·상위 폴더 탈출·인코딩된 경로 탈출·로컬 경계를 벗어나는 링크를 거절한다. glTF URI는 percent decode 후 검사하고 상위 경로 조각을 허용하지 않는다. 네트워크 요청과 자동 다운로드는 없다. 최대16MiB 모델/버퍼 입력·32MiB 자원 예약, PNG/JPEG8MiB/변1024/디코드16MiB, 노드512·primitive128·관절128·재질256·정점65536/모델·삼각형131072/모델·문서 모델256·native 장면4MiB와 전체 사본/Undo/텍스처 메모리 예산을 적용한다. 유효하지 않은 결과는 확정하지 않는다.

형식 해석은 [Khronos glTF2 공식 명세](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/Specification.adoc)의 좌표/버퍼/접근자/재질/스키닝 규칙을 바탕으로 한 명시적 부분 지원이다. 모든 확장·모든 원본 애플리케이션과의 완전 호환을 주장하지 않는다. 원본 SAI/CSP 코드·자산을 사용하지 않는다.
