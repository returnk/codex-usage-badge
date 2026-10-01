param([int]$BadgeProcessId,[string]$Name='baseline',[ValidateSet('Right','Left')][string]$Side='Right',[int]$Cycles=6,[switch]$AlreadyOpen)
$ErrorActionPreference='Stop'
& "$PSScriptRoot/avatar-layout-desktop.ps1" -Action $(if($AlreadyOpen){'Observe'}else{'Menu'}) -BadgeProcessId $BadgeProcessId | Out-Null
$taskMenu=@([AvatarLayoutDesktop]::Windows($BadgeProcessId)|Where-Object {$_.Title -eq 'Codex Badge Menu' -and $_.Visible})[0]
$taskScale=$taskMenu.Dpi/96.0
$taskRootX=$taskMenu.X;$taskRootY=$taskMenu.Y
$taskDir=Join-Path $PSScriptRoot "../artifacts/rendering-2026-10-01/$Name"
[void][IO.Directory]::CreateDirectory($taskDir)
$taskRows=@()
for($taskCycle=0;$taskCycle -lt $Cycles;$taskCycle++) {
    foreach($taskRow in @(58,96)) {
        [void][AvatarLayoutDesktop]::SetCursorPos($taskRootX+[int](80*$taskScale),$taskRootY+[int]($taskRow*$taskScale))
        for($taskFrame=0;$taskFrame -lt 20;$taskFrame++) {
            $taskCurrent=@([AvatarLayoutDesktop]::Windows($BadgeProcessId)|Where-Object {$_.Title -eq 'Codex Badge Menu'})[0]
            $taskExpectedX=if($Side -eq 'Left' -and $taskCurrent.W -gt $taskMenu.W){$taskRootX-[int](184*$taskScale)}else{$taskRootX}
            $taskBitmap=[Drawing.Bitmap]::new($taskMenu.W,$taskMenu.H)
            $taskGraphics=[Drawing.Graphics]::FromImage($taskBitmap)
            try {
                $taskGraphics.CopyFromScreen($taskRootX,$taskRootY,0,0,[Drawing.Size]::new($taskMenu.W,$taskMenu.H))
                $taskBlack=0;$taskSamples=0
                for($taskY=15;$taskY -lt $taskMenu.H-15;$taskY+=4){for($taskX=15;$taskX -lt $taskMenu.W-15;$taskX+=4){
                    $taskPixel=$taskBitmap.GetPixel($taskX,$taskY);$taskSamples++
                    if($taskPixel.R -lt 20 -and $taskPixel.G -lt 20 -and $taskPixel.B -lt 20){$taskBlack++}
                }}
                $taskBitmap.Save((Join-Path $taskDir "$taskCycle-$taskRow-$taskFrame.png"),[Drawing.Imaging.ImageFormat]::Png)
                $taskRows+=[pscustomobject]@{Cycle=$taskCycle;Row=$taskRow;Frame=$taskFrame;Visible=$taskCurrent.Visible;X=$taskCurrent.X;Y=$taskCurrent.Y;W=$taskCurrent.W;Stable=($taskCurrent.X -eq $taskExpectedX -and $taskCurrent.Y -eq $taskRootY);BlackRatio=$taskBlack/$taskSamples}
            }finally{$taskGraphics.Dispose();$taskBitmap.Dispose()}
            Start-Sleep -Milliseconds 10
        }
    }
}
$taskRows|ConvertTo-Json|Set-Content (Join-Path $taskDir 'frames.json')
[pscustomobject]@{Frames=$taskRows.Count;Unstable=@($taskRows|Where-Object {!$_.Stable}).Count;Hidden=@($taskRows|Where-Object {!$_.Visible}).Count;MaxBlackRatio=($taskRows.BlackRatio|Measure-Object -Maximum).Maximum;Evidence=$taskDir}|ConvertTo-Json
& "$PSScriptRoot/avatar-layout-desktop.ps1" -Action Leave -BadgeProcessId $BadgeProcessId | Out-Null
