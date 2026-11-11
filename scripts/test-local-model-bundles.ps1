param([string]$Evidence='docs/process/local-model-bundles-evidence.json')
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$base=Join-Path $repo ('.verify-280-models-'+[guid]::NewGuid().ToString('N'))
$data=Join-Path $base 'userdata';$receipt=Join-Path $base 'receipt.txt'
$oldData=$env:DRAWER_DATA_DIR;$oldDiag=$env:DRAWER_SMOKE_DIAGNOSTICS;$child=$null
try {
    [void](New-Item -ItemType Directory -Path $base)
    $env:DRAWER_DATA_DIR=$data;$env:DRAWER_SMOKE_DIAGNOSTICS=$base
    $exe=Join-Path $repo 'dist/Drawer-windows-x64/Drawer.exe'
    $child=Start-Process -FilePath $exe -ArgumentList @('--smoke-test',('"'+$receipt+'"')) -WindowStyle Hidden -PassThru
    if(-not $child.WaitForExit(90000) -or $child.ExitCode -ne 0){if(Test-Path -LiteralPath (Join-Path $base 'smoke-failure.txt')){Get-Content -LiteralPath (Join-Path $base 'smoke-failure.txt')};throw '로컬 모델 설치본 시험 실패'}
    $text=Get-Content -LiteralPath $receipt -Raw
    foreach($flag in 'local_inference=true','local_model_bundles=true','error_reporting_disabled=true','saved_state=true'){if(-not $text.Contains($flag)){throw '로컬 모델 영수증 누락'}}
    $user=Join-Path $data 'inference-user'
    foreach($entry in @(Get-Item -LiteralPath $user -Force)+@(Get-ChildItem -LiteralPath $user -Recurse -Force)){if($entry.Attributes -band [IO.FileAttributes]::ReparsePoint){throw '검증 저장소 링크 거절'}}
    $target=Join-Path $repo $Evidence;$sha=(Get-FileHash -LiteralPath $exe).Hash.ToLowerInvariant()
@'
import hashlib,json,re,sys
from pathlib import Path
root=Path(sys.argv[1]);output=Path(sys.argv[2]);exe_sha=sys.argv[3]
registry_path=root/'settings/local-models.json'
registry_bytes=registry_path.read_bytes()
registry=json.loads(registry_bytes)
assert registry['version']==1
assert sorted(r['task'] for r in registry['entries'])==sorted(['Body','Hand','HintColor','Upscale','Denoise'])
models=[]
for record in registry['entries']:
 assert set(record)=={'task','manifest_sha256','name','version','license'}
 pin=record['manifest_sha256'];assert re.fullmatch('[0-9a-f]{64}',pin)
 folder=root/'models'/pin
 assert sorted(p.name for p in folder.iterdir())==['LICENSE.txt','manifest.json','weights.onnx']
 raw=(folder/'manifest.json').read_bytes();assert hashlib.sha256(raw).hexdigest()==pin
 manifest=json.loads(raw);weights=(folder/'weights.onnx').read_bytes();license_bytes=(folder/'LICENSE.txt').read_bytes()
 assert manifest['format']==1 and manifest['task']==record['task'] and manifest['license']==record['license']
 assert 0<len(raw)<=16384 and 0<len(weights)<=64*1024*1024 and 0<len(license_bytes)<=65536
 assert hashlib.sha256(weights).hexdigest()==manifest['weights_sha256']
 assert hashlib.sha256(license_bytes).hexdigest()==manifest['license_sha256']
 assert license_bytes.decode('utf-8-sig').strip()
 assert 1<=manifest['width']<=512 and 1<=manifest['height']<=512
 models.append({'task':record['task'],'manifestSha256':pin,'weightsBytes':len(weights),'weightsSha256':manifest['weights_sha256'],'licenseSha256':manifest['license_sha256'],'licenseIdentifier':record['license']})
assert not list((root/'models').glob('.drawer-model-*'))
assert not list(root.rglob('*.tmp'))
assert not list(root.glob('drawer-infer-*'))
result={'version':1,'executableSha256':exe_sha,'registrySha256':hashlib.sha256(registry_bytes).hexdigest(),'relativeRegistrations':True,'models':models,'consentBeforeCopyAndRun':True,'actualCpuHelperAndUiReplay':True,'originalDrawingAndUndoPreserved':True,'scratchResidue':0,'limits':'Self-authored deterministic ONNX fixtures exercise adapter behavior; no third-party weights are shipped. Fresh offline-PC272 remains separate.'}
output.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print('Five registered cache bundles independently verified: manifest/weights/license SHA256, dimensions, no scratch residue')
'@ | python -X utf8 -B - $user $target $sha
    if($LASTEXITCODE -ne 0){throw '실제 모델 캐시/등록 파일 독립 대조 실패'}
} finally {
    if($child -and -not $child.HasExited){$child.Kill();$child.WaitForExit()}
    $env:DRAWER_DATA_DIR=$oldData;$env:DRAWER_SMOKE_DIAGNOSTICS=$oldDiag
    if(Test-Path -LiteralPath $base){
        if(-not ([IO.Path]::GetFullPath($base)).StartsWith($repo+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)){throw '시험 삭제 범위 오류'}
        foreach($entry in @(Get-Item -LiteralPath $base)+@(Get-ChildItem -LiteralPath $base -Recurse -Force)){if($entry.Attributes -band [IO.FileAttributes]::ReparsePoint){throw '시험 링크 보존'}}
        Remove-Item -LiteralPath $base -Recurse -Force
    }
}
