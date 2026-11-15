$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'process-memory.ps1')
$samples=@(1..600|ForEach-Object {[ordered]@{seconds=[double]$_;privateBytes=30MB;workingSetBytes=40MB}})
$r=Get-SoakMemorySummary -Samples $samples
if($r.warmMeanPrivateBytes -ne 30MB -or $r.lateMeanPrivateBytes -ne 30MB -or $r.deltaPrivateBytes -ne 0){throw 'Dictionary means incorrect'}
$objects=@($samples|ForEach-Object {[pscustomobject]$_})
if((Get-SoakMemorySummary -Samples $objects).warmMeanPrivateBytes -ne 30MB){throw 'Object means incorrect'}
foreach($case in 'growth','missing','zero','time','short'){
    $bad=@($samples|ForEach-Object {[ordered]@{seconds=$_.seconds;privateBytes=$_.privateBytes;workingSetBytes=$_.workingSetBytes}})
    switch($case){
        growth{foreach($s in $bad){if($s.seconds -ge 480){$s.privateBytes=100MB}}}
        missing{$bad[0].privateBytes=$null}
        zero{$bad[0].privateBytes=0}
        time{$bad[1].seconds=0}
        short{$bad=@($bad|Select-Object -First 400)}
    }
    $failed=$false
    try{[void](Get-SoakMemorySummary -Samples $bad)}catch{$failed=$true}
    if(-not $failed){throw ('Invalid fixture accepted: '+$case)}
}
Write-Output 'Process memory dictionary/object means and5 invalid fixtures passed'
