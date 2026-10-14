# Drawer 실행·설치·제거

Windows 64비트에서 `Drawer.exe`를 실행한다. 기본 도구와 글꼴을 실행 파일에 포함해 별도 소재 다운로드가 필요하지 않다.

사용자 계정에 설치하려면 배포 폴더에서 PowerShell로 `./Install.ps1`을 실행한다. 기본 설치 위치는 사용자 LocalAppData의 `Programs/Drawer`이며 관리자 권한을 요구하지 않는다.

설정은 사용자 LocalAppData의 `Drawer/settings`, 사용자 소재는 `Drawer/materials`에 보관한다. 문서는 사용자가 선택한 위치에 보관한다.

제거하려면 설치 폴더의 `./Uninstall.ps1`을 실행한다. 설치 기록에 있는 프로그램 파일만 제거하며 사용자 문서·소재·설정과 추가 파일을 보존한다. 재설치도 이 데이터를 삭제하지 않는다.
