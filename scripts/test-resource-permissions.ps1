param([string]$Executable='dist/Drawer-windows-x64/Drawer.exe')
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$base=[IO.Path]::GetFullPath((Join-Path $repo '.verify-270-permissions'))
if (-not $base.StartsWith($repo+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { throw '시험 경로 범위 오류' }
if(Test-Path -LiteralPath $base){throw '기존 권한 시험 폴더는 보존합니다.'}
[void](New-Item -ItemType Directory -Path $base)
$scratch=Join-Path $base 'scratch'
[void](New-Item -ItemType Directory -Path $scratch)
$receipt=Join-Path $base 'receipt.txt'
$previous=Join-Path $scratch 'previous.guard'
[IO.File]::WriteAllBytes($previous,[byte[]](1,2,3,4))
$beforeHash=(Get-FileHash -LiteralPath $previous -Algorithm SHA256).Hash
$original=Get-Acl -LiteralPath $scratch
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User
try {
    $acl=Get-Acl -LiteralPath $scratch
    $deny=[Security.AccessControl.FileSystemAccessRule]::new($sid,[Security.AccessControl.FileSystemRights]::Write,[Security.AccessControl.InheritanceFlags]'ContainerInherit,ObjectInherit',[Security.AccessControl.PropagationFlags]::None,[Security.AccessControl.AccessControlType]::Deny)
    [void]$acl.AddAccessRule($deny)
    Set-Acl -LiteralPath $scratch -AclObject $acl
    $exe=(Resolve-Path -LiteralPath (Join-Path $repo $Executable)).Path
    $probeArguments=@('--verify-scratch-denied',('"'+$scratch+'"'),('"'+$receipt+'"'))
    $child=Start-Process -FilePath $exe -ArgumentList $probeArguments -WindowStyle Hidden -Wait -PassThru
    if($child.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $receipt) -or (Get-Content -LiteralPath $receipt -Raw).Trim() -ne 'permission_revoked=true'){throw '실제 쓰기 권한 철회 검증 실패'}
    if((Get-FileHash -LiteralPath $previous -Algorithm SHA256).Hash -ne $beforeHash -or @(Get-ChildItem -LiteralPath $scratch -Force).Count -ne 1){throw '권한 철회 후 이전 바이트 변경 또는 임시 파일 잔류'}
    Write-Output '실제 Windows 쓰기 권한 철회: 설치 패키지의 probe 거절 확인; 정상 파일 보호.'
} finally {
    Set-Acl -LiteralPath $scratch -AclObject $original
    $entries=@(Get-ChildItem -LiteralPath $base -Recurse -Force)
    foreach($entry in $entries){if($entry.Attributes -band [IO.FileAttributes]::ReparsePoint){throw '시험 폴더 링크 보존'};if(-not $entry.FullName.StartsWith($base+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw '시험 경로 범위 오류'}}
    Remove-Item -LiteralPath $base -Recurse -Force
}
