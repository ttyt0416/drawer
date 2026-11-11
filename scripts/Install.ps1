param(
    [string]$InstallDir=(Join-Path $env:LOCALAPPDATA 'Programs/Drawer'),
    [Parameter(Mandatory=$true)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExpectedManifestSha256,
    [switch]$AllowUnsigned,
    [ValidatePattern('^$|^[a-fA-F0-9]{64}$')][string]$ExpectedPublisherSha256='',
    [switch]$AllowDowngrade
)
$ErrorActionPreference='Stop'
# Cache-only trust: no certificate, CRL or AIA retrieval.
if(-not ('DrawerOfflineTrust' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class DrawerOfflineTrust {
 [StructLayout(LayoutKind.Sequential)] struct F {public uint size;public IntPtr path,file,subject;}
 [StructLayout(LayoutKind.Sequential)] struct D {public uint size;public IntPtr policy,sip;public uint ui,revocation,choice;public IntPtr file;public uint action;public IntPtr state,url;public uint flags,context;}
 [DllImport("wintrust.dll",ExactSpelling=true)] static extern int WinVerifyTrust(IntPtr hwnd,ref Guid action,ref D data);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode)] public static extern uint GetDriveType(string root);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool GetDiskFreeSpaceEx(string root,out ulong available,out ulong total,out ulong free);
 public static ulong Available(string root){ulong a,t,f;if(!GetDiskFreeSpaceEx(root,out a,out t,out f))throw new System.ComponentModel.Win32Exception();return a;}
 public static int Verify(string path){
  IntPtr name=Marshal.StringToCoTaskMemUni(path),info=Marshal.AllocHGlobal(Marshal.SizeOf(typeof(F)));
  try {var f=new F{size=(uint)Marshal.SizeOf(typeof(F)),path=name};Marshal.StructureToPtr(f,info,false);
   var d=new D{size=(uint)Marshal.SizeOf(typeof(D)),ui=2,choice=1,file=info,flags=0x1010};
   var action=new Guid("00AAC56B-CD44-11D0-8CC2-00C04FC295EE");return WinVerifyTrust(new IntPtr(-1),ref action,ref d);
  } finally {Marshal.FreeHGlobal(info);Marshal.FreeCoTaskMem(name);}
 }
}
'@
}
function Local-Path([string]$Path) {
    $absolute=[IO.Path]::GetFullPath($Path);$plain=$absolute
    if($plain.StartsWith('\\?\')){$plain=$plain.Substring(4)}
    if($plain -notmatch '^[A-Za-z]:[\\/]' -or $plain.Contains('://') -or $plain.Substring(2).Contains(':')){throw '로컬 드라이브 경로를 선택하세요.'}
    if([DrawerOfflineTrust]::GetDriveType($plain.Substring(0,3)) -notin 2,3,5,6){throw '네트워크/연결 해제 드라이브 거절'}
    $current=$plain.Substring(0,3)
    foreach($part in $plain.Substring(3).Split([char[]]'\/',[StringSplitOptions]::RemoveEmptyEntries)) {
        if($part -in '.','..'){throw '상위 경로 탈출 거절'}
        $current=Join-Path $current $part
        try {$entry=Get-Item -LiteralPath $current -Force -ErrorAction Stop} catch [System.Management.Automation.ItemNotFoundException] {break}
        if($entry.Attributes -band [IO.FileAttributes]::ReparsePoint){throw '설치는 링크/재분석 지점을 따르지 않습니다.'}
    }
    return $absolute
}
function File-Hash([string]$Path) {
    $input=[IO.File]::OpenRead($Path);$sha=[Security.Cryptography.SHA256]::Create()
    try {return [BitConverter]::ToString($sha.ComputeHash($input)).Replace('-','')} finally {$sha.Dispose();$input.Dispose()}
}
function Has-EmbeddedSignature([string]$Path) {
    $payloadStream=[IO.File]::OpenRead($Path);$binary=[IO.BinaryReader]::new($payloadStream)
    try {
        if($payloadStream.Length -lt 256 -or $binary.ReadUInt16() -ne 0x5A4D){throw '잘못된 실행 파일'}
        $payloadStream.Position=60;$pe=$binary.ReadUInt32()
        if($pe -gt $payloadStream.Length-160){throw '잘못된 PE 머리말'}
        $payloadStream.Position=$pe;if($binary.ReadUInt32() -ne 0x4550){throw '잘못된 PE 서명'}
        $optional=$pe+24;$payloadStream.Position=$optional;$magic=$binary.ReadUInt16()
        if($magic -eq 0x20b){$directory=112}elseif($magic -eq 0x10b){$directory=96}else{throw '지원하지 않는 실행 파일'}
        $payloadStream.Position=$optional+$directory-4
        if($binary.ReadUInt32() -lt 5){return $false}
        $payloadStream.Position=$optional+$directory+32;$offset=$binary.ReadUInt32();$size=$binary.ReadUInt32()
        if($offset -eq 0 -and $size -eq 0){return $false}
        if($offset -eq 0 -or $size -lt 8 -or $offset%8 -ne 0 -or ([uint64]$offset+$size) -gt $payloadStream.Length){throw '손상된 실행 파일 서명 표'}
        return $true
    } finally {$binary.Dispose();$payloadStream.Dispose()}
}
function Tree([string]$Root) {
    $records=@{};$pending=[Collections.Generic.Stack[string]]::new();$pending.Push($Root);[uint64]$bytes=0
    while($pending.Count){
        foreach($entry in @(Get-ChildItem -LiteralPath $pending.Pop() -Force)) {
            if($entry.Attributes -band [IO.FileAttributes]::ReparsePoint){throw '설치 트리 링크는 보존하고 중단합니다.'}
            $rel=$entry.FullName.Substring($Root.TrimEnd([char[]]'\/').Length+1)
            if($records.Count -ge 10000 -or $rel.Length -gt 4096){throw '설치 트리 한도 초과'}
            if($entry.PSIsContainer){$records[$rel]='directory';$pending.Push($entry.FullName)}
            else {$bytes+=$entry.Length;if($bytes -gt 16GB){throw '설치 트리16GiB 한도'};$records[$rel]=[string]$entry.Length+':'+(File-Hash $entry.FullName)}
        }
    }
    return @{files=$records;bytes=$bytes}
}
function Same-Tree($A,$B) {
    if($A.Count -ne $B.Count){return $false};foreach($key in $A.Keys){if(-not $B.ContainsKey($key) -or $A[$key] -cne $B[$key]){return $false}};return $true
}
$package=Local-Path $PSScriptRoot
$installationRoot=Local-Path $InstallDir
if($installationRoot -eq [IO.Path]::GetPathRoot($installationRoot) -or $installationRoot -eq $package){throw '별도의 설치 폴더를 선택하세요.'}
$manifestPath=Local-Path (Join-Path $package 'PACKAGE.json')
if((File-Hash $manifestPath) -ine $ExpectedManifestSha256){throw '배포 기록과 패키지 명세 SHA256 불일치'}
$manifest=Get-Content -LiteralPath $manifestPath -Raw|ConvertFrom-Json
$files=@('Drawer.exe','THIRD-PARTY-NOTICES.txt','COMPONENTS.json','Cargo.lock','OWN-ASSETS.md','START-HERE.md','MOVIE-IMPORT.md','IMAGE-IMPORT.md','HELP.md','MODEL-FORMATS.md','Install.ps1','Uninstall.ps1')
if($manifest.format -ne 1 -or $manifest.product -cne 'Drawer' -or $manifest.settingsVersion -ne 1 -or $manifest.nativeMin -ne 1 -or $manifest.nativeMax -lt 1 -or @($manifest.files).Count -ne $files.Count){throw '지원하지 않는 배포 명세'}
$version=[Version]$manifest.version;$seen=@{};[uint64]$packageBytes=0
foreach($record in $manifest.files) {
    if($record.name -notin $files -or $seen.ContainsKey($record.name) -or $record.sha256 -notmatch '^[a-fA-F0-9]{64}$' -or $record.bytes -lt 1 -or $record.bytes -gt 2GB){throw '중복/잘못된 파일 명세'}
    $seen[$record.name]=$true;$path=Local-Path (Join-Path $package $record.name)
    if((Get-Item -LiteralPath $path).Length -ne $record.bytes -or (File-Hash $path) -ine $record.sha256){throw "설치 파일 길이/해시 불일치: $($record.name)"}
    $packageBytes+=$record.bytes
}
$declaredSignature=Has-EmbeddedSignature (Join-Path $package 'Drawer.exe')
$trust=[DrawerOfflineTrust]::Verify((Join-Path $package 'Drawer.exe'))
if($trust -eq -2146762496) { # TRUST_E_NOSIGNATURE
    if($declaredSignature){throw '내장 서명이 선언되었지만 검증할 수 없습니다. unsigned 허용으로 우회하지 않습니다.'}
    if(-not $AllowUnsigned){throw '서명 없는 로컬 빌드입니다. 검토한 명세 SHA256과 -AllowUnsigned를 명시하세요.'}
    Write-Output '서명 없는 로컬 빌드: 지정한 해시를 검증했습니다. 배포자 서명을 주장하지 않습니다.'
} elseif($trust -ne 0) {throw ('오프라인 서명 검증 실패: 0x'+$trust.ToString('X8'))}
else {
    if(-not $ExpectedPublisherSha256){throw '확인한 배포자 인증서 SHA256을 명시하세요.'}
    $cert=[Security.Cryptography.X509Certificates.X509Certificate2]::new([Security.Cryptography.X509Certificates.X509Certificate]::CreateFromSignedFile((Join-Path $package 'Drawer.exe')))
    $sha=[Security.Cryptography.SHA256]::Create()
    try {if([BitConverter]::ToString($sha.ComputeHash($cert.RawData)).Replace('-','') -ine $ExpectedPublisherSha256){throw '배포자 인증서 불일치'}} finally {$sha.Dispose();$cert.Dispose()}
}
$exists=Test-Path -LiteralPath $installationRoot
$before=@{files=@{};bytes=0}
if($exists) {
    if(-not (Get-Item -LiteralPath $installationRoot).PSIsContainer){throw '설치 대상이 폴더가 아닙니다.'}
    $before=Tree $installationRoot;$markerPath=Join-Path $installationRoot '.drawer-install.json'
    if($before.files.Count -and -not (Test-Path -LiteralPath $markerPath)){throw '소유 기록 없는 기존 폴더는 보존합니다.'}
    if(Test-Path -LiteralPath $markerPath){
        $previous=Get-Content -LiteralPath $markerPath -Raw|ConvertFrom-Json
        if($previous.product -ne 'Drawer' -or $previous.installRoot -ne $installationRoot){throw '이전 설치 기록 불일치'}
        if($previous.version -and ([Version]$previous.version) -gt $version -and -not $AllowDowngrade){throw '이전 버전은 -AllowDowngrade 확인이 필요합니다. 새 문서 형식은 원본을 유지하고 읽기를 거절할 수 있습니다.'}
    }
}
$parent=Split-Path -Parent $installationRoot;$probe=$parent
while(-not (Test-Path -LiteralPath $probe)){$probe=Split-Path -Parent $probe}
if([DrawerOfflineTrust]::Available($probe) -lt ($before.bytes+$packageBytes+1MB)){throw '설치/이전 정상본 보존 공간 부족; 기존 앱 유지'}
[void][IO.Directory]::CreateDirectory($parent)
$id=[guid]::NewGuid().ToString('N');$stage=Join-Path $parent ('.Drawer-stage-'+$id);$retained=Join-Path $parent ('.Drawer-previous-'+$id)
$stageTree=$null;$oldMoved=$false
try {
    [void](New-Item -ItemType Directory -Path $stage)
    if($exists){foreach($entry in @(Get-ChildItem -LiteralPath $installationRoot -Force)){Copy-Item -LiteralPath $entry.FullName -Destination $stage -Recurse -Force}}
    foreach($name in $files+@('PACKAGE.json')){Copy-Item -LiteralPath (Join-Path $package $name) -Destination (Join-Path $stage $name) -Force}
    foreach($record in $manifest.files){if((File-Hash (Join-Path $stage $record.name)) -ine $record.sha256){throw '준비 복사본 해시 오류'}}
    if((File-Hash (Join-Path $stage 'PACKAGE.json')) -ine $ExpectedManifestSha256){throw '준비 명세 해시 오류'}
    [ordered]@{product='Drawer';format=2;version=$manifest.version;nativeMax=$manifest.nativeMax;installRoot=$installationRoot;files=$files+@('PACKAGE.json');manifestSha256=$ExpectedManifestSha256}|ConvertTo-Json -Depth 4|Set-Content -LiteralPath (Join-Path $stage '.drawer-install.json') -Encoding utf8
    $stageTree=Tree $stage
    if($exists){if(-not (Same-Tree $before.files (Tree $installationRoot).files)){throw '업데이트 중 기존 설치 변경; 보존합니다.'};Move-Item -LiteralPath $installationRoot -Destination $retained;$oldMoved=$true}
    if(Test-Path -LiteralPath $installationRoot){throw '목적지가 새로 나타났습니다. 기존 자료를 덮지 않습니다.'}
    Move-Item -LiteralPath $stage -Destination $installationRoot
    Write-Output "Drawer $version 설치 완료: $installationRoot"
    if($oldMoved){Write-Output "이전 정상 앱/추가 파일 보존: $retained"}
    Write-Output "Native $($manifest.nativeMin)..$($manifest.nativeMax); 설정/문서/소재 저장소 유지. 자동 접속/통계/오류 전송 없음."
} catch {
    $failure=$_
    if($oldMoved -and -not (Test-Path -LiteralPath $installationRoot)){Move-Item -LiteralPath $retained -Destination $installationRoot;$oldMoved=$false}
    if(Test-Path -LiteralPath $stage){
        if($stageTree -and (Same-Tree $stageTree.files (Tree $stage).files)){Remove-Item -LiteralPath $stage -Recurse -Force}
        else {Write-Output "검증되지 않은 준비 복사본 보존: $stage"}
    }
    if($oldMoved){Write-Output "이전 정상 앱 복구 위치: $retained"}
    throw $failure
}
