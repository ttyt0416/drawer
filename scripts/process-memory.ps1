function Get-SoakMemorySummary {
    param([Parameter(Mandatory)]$Samples)
    $rows=@($Samples)
    if($rows.Count -lt 500){throw 'Incomplete memory sample count'}
    $previous=-1.0
    foreach($row in $rows){
        if($null -eq $row.seconds -or $null -eq $row.privateBytes -or $null -eq $row.workingSetBytes){throw 'Missing memory sample value'}
        if(([double]::IsNaN([double]$row.seconds) -or [double]::IsInfinity([double]$row.seconds)) -or [double]$row.seconds -le $previous -or [long]$row.privateBytes -le 0 -or [long]$row.workingSetBytes -le 0){throw 'Invalid memory sample'}
        $previous=[double]$row.seconds
    }
    $warm=@($rows|Where-Object {$_.seconds -ge 60 -and $_.seconds -le 180})
    $late=@($rows|Where-Object {$_.seconds -ge 480})
    if(-not $warm.Count -or -not $late.Count){throw 'Missing process memory intervals'}
    # Extract dictionary values explicitly; Measure-Object cannot infer dictionary keys.
    $warmMean=($warm|ForEach-Object {[double]$_.privateBytes}|Measure-Object -Average).Average
    $lateMean=($late|ForEach-Object {[double]$_.privateBytes}|Measure-Object -Average).Average
    if($null -eq $warmMean -or $null -eq $lateMean -or [double]::IsNaN($warmMean) -or [double]::IsInfinity($warmMean) -or [double]::IsNaN($lateMean) -or [double]::IsInfinity($lateMean)){throw 'Missing/invalid process memory means'}
    if($lateMean-$warmMean -gt 64MB){throw 'Private memory increased beyond64MiB after warmup'}
    return [ordered]@{scope='Windows private bytes and working set; CPU-only process';samples=$rows;warmMeanPrivateBytes=$warmMean;lateMeanPrivateBytes=$lateMean;deltaPrivateBytes=$lateMean-$warmMean;allowedGrowthBytes=64MB;stable=$true}
}
