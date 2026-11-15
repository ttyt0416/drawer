# 독자 시험 그림 (302)

`Drawer.exe --verify-item 302 NEW_DIRECTORY`는 외부 자산 없이 소스의 수학식·좌표로 64×48 반투명 배경 위에 straight/circle/corner/very-slow/fast/pause/lift-gap의7개 입력 JSON과 PNG·Native 그림을 생성한다. `scripts/test-checklist.ps1 -Item 302`는 같은 검사를 실행하고 증거 JSON을 남긴 후 자기 임시 그림을 삭제한다. 그림을 보관하려면 새 로컬 폴더로 직접 실행한다.

모든 그림은 시작/끝 픽셀이 바뀌고 맨 위·아래 행은 원래 RGBA와 정확히 같아야 한다. 원의 중앙은 비어 있고 lift-gap의 가운데 행은 두 획이 연결되지 않아야 한다. 느린 획은100ms 간격, 빠른 획은 같은 시각, 정지는 같은 점의1초 간격이다. 실제 대기/실물 지연 시험과 구분한다. 정확 반복·한/두 획 Undo/Redo·접촉 정리·PNG RGBA 재열기를 검사한다.

추가로 선택/알파 잠금/마스크와 타일 경계 검증290을 실행하고, 0길이 선화의 반투명 점, 1×32768 이미지 회전의 양 끝, 1e-9 이동 Undo, 최대256개 레이어 합성/병합/한 Undo를 검사한다. 출처는 `verification_pictures.rs`·`verification_strokes.rs`이며 원본 제품 그림·코드·리소스를 사용하지 않는다.
