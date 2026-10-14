# Drawer

완전 로컬 Windows x64 그림 프로그램을 Rust로 구현하는 저장소다.

개발 순서와 완료 조건은 [기능 체크리스트](docs/specs/feature-checklist.md)에 있다. 번호 하나를 검증할 때마다 체크리스트를 갱신하고 별도 커밋으로 푸시한다.

구현 입력은 기능 명세다. 조사 원문·원본 바이너리·추출 리소스·디컴파일 산출물은 이 저장소 밖에 보관한다. [분리 규칙](docs/process/clean-room.md)과 `handoff-files.txt`가 구현 전달 범위를 정한다.

```powershell
cargo test --workspace --offline --target-dir target/latest
cargo run -p xtask --offline --target-dir target/latest -- prepare-handoff
```

`prepare-handoff`는 목록에 있는 텍스트 파일만 `handoff/latest`에 복사하고 전달 목록을 검사한다. 빌드 결과와 전달 묶음은 Git에 포함하지 않는다.

앱 실행: `cargo run -p drawer-app --offline --target-dir target/latest`.

여러 로컬 `.drawer` 문서, PNG/JPEG/TIFF에서 독립 새 그림 만들기, 이미지 클립보드와 최근 파일을 지원한다. 문서별 도구·이력과 같은 그림의 여러 보기를 유지한다. 실행 방법은 [사용 안내](docs/START-HERE.md), 파일 보존 범위는 [문서 형식](docs/process/native-format.md)에 있다.
