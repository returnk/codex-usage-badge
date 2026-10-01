param(
    [ValidateSet('Observe','FocusHost','Capsule','Detail','CreditButton','CreditHover','Credit','Leave','Menu','Topmost','Settings','SettingsHover','NotificationHover','CloseSettings','Startup','DoubleClick','Notifications','Sidebar','ResizeSidebar','DragCapsule','RapidHover','Move','RestoreGeometry','Maximize','Restore','Minimize')]
    [string]$Action = 'Observe',
    [string]$Capture,
    [int]$CaptureWidth = 680,
    [int]$CaptureHeight = 410,
    [int]$BadgeProcessId = 0,
    [string]$CaptureWindow,
    [ValidateSet('Right','Left')][string]$SubmenuSide='Right',
    [int[]]$Delta = @(40,-40),
    [int[]]$Geometry
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes,System.Drawing
if (-not ('AvatarLayoutDesktop' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class AvatarLayoutDesktop {
    public delegate bool EnumProc(IntPtr h,IntPtr p);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int L,T,R,B; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f,IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int n);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h,IntPtr after,int x,int y,int w,int height,uint flags);
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [StructLayout(LayoutKind.Sequential)] public struct Cursor { public int Size,Flags;public IntPtr Handle;public int X,Y; }
    [DllImport("user32.dll")] public static extern bool GetCursorInfo(ref Cursor c);
    [DllImport("user32.dll")] public static extern IntPtr LoadCursor(IntPtr instance,IntPtr id);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
    [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f,uint x,uint y,uint data,UIntPtr extra);
    public static object[] Windows(uint pid) {
        var rows=new List<object>();
        EnumWindows((h,p)=>{uint id;GetWindowThreadProcessId(h,out id);if(id!=pid)return true;
            Rect r;GetWindowRect(h,out r);var title=new StringBuilder(128);GetWindowText(h,title,128);
            rows.Add(new {Hwnd=h.ToInt64(),Title=title.ToString(),Visible=IsWindowVisible(h),
                X=r.L,Y=r.T,W=r.R-r.L,H=r.B-r.T,Dpi=GetDpiForWindow(h)});return true;},IntPtr.Zero);
        return rows.ToArray();
    }
}
'@
}
[void][AvatarLayoutDesktop]::SetThreadDpiAwarenessContext([IntPtr](-4))
$taskHost = @(Get-Process -Name ChatGPT | Where-Object {$_.MainWindowHandle -ne 0})
if ($taskHost.Count -ne 1) { throw 'Expected one live client window' }
$taskHwnd = $taskHost[0].MainWindowHandle
$taskBadge = if ($BadgeProcessId) { Get-Process -Id $BadgeProcessId } else { Get-Process -Name codex-badge-tauri }
$taskWindows = @([AvatarLayoutDesktop]::Windows($taskBadge.Id))
$taskRoot = [System.Windows.Automation.AutomationElement]::FromHandle($taskHwnd)
function Find-Button($root,$name) {
    $condition=[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,$name)
    $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants,$condition)
}
function Find-Sidebar($root) {
    $parent=Find-Button $root '打开个人资料菜单'
    $walker=[System.Windows.Automation.TreeWalker]::RawViewWalker
    for($depth=0;$depth -lt 8 -and $parent;$depth++) {
        if($parent.Current.AutomationId -eq 'app-shell-sidebar'){return $parent}
        $parent=$walker.GetParent($parent)
    }
}
function Invoke-Button($element) {
    if (!$element) { throw 'Expected button absent' }
    $r=$element.Current.BoundingRectangle
    if($r.IsEmpty -or $element.Current.IsOffscreen){throw 'Button is not visible'}
    [void][AvatarLayoutDesktop]::SetCursorPos([int]($r.X+$r.Width/2),[int]($r.Y+$r.Height/2))
    [AvatarLayoutDesktop]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    [AvatarLayoutDesktop]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
}
function Move-To($title) {
    $window=@($taskWindows | Where-Object {$_.Title -eq $title -and $_.Visible})
    if ($window.Count -ne 1) { throw "Expected visible $title" }
    [void][AvatarLayoutDesktop]::SetCursorPos($window[0].X+[int]($window[0].W/2),$window[0].Y+[int]($window[0].H/2))
}
function Drag-Point($x,$y,$dx,$dy) {
    [void][AvatarLayoutDesktop]::SetCursorPos($x,$y)
    [AvatarLayoutDesktop]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    for($step=1;$step -le 10;$step++) {
        [void][AvatarLayoutDesktop]::SetCursorPos([int]($x+$dx*$step/10),[int]($y+$dy*$step/10))
        Start-Sleep -Milliseconds 25
    }
    Start-Sleep -Milliseconds 250
    [AvatarLayoutDesktop]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
}
function Click-Popup($title,$x,$y) {
    $taskPopup=@($taskWindows | Where-Object {$_.Title -eq $title -and $_.Visible})
    if($taskPopup.Count -ne 1){throw "Popup absent: $title"}
    $taskScale=$taskPopup[0].Dpi/96
    [void][AvatarLayoutDesktop]::SetCursorPos($taskPopup[0].X+[int]($x*$taskScale),$taskPopup[0].Y+[int]($y*$taskScale))
    Start-Sleep -Milliseconds 70
    [AvatarLayoutDesktop]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
    [AvatarLayoutDesktop]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
}
switch ($Action) {
    FocusHost { [void][AvatarLayoutDesktop]::SetForegroundWindow($taskHwnd) }
    Capsule { Move-To 'Codex Badge' }
    Detail { Move-To 'Codex Badge Detail' }
    Credit { Move-To 'Codex Badge Reset Credits' }
    Leave { $r=[AvatarLayoutDesktop+Rect]::new();[void][AvatarLayoutDesktop]::GetWindowRect($taskHwnd,[ref]$r);[void][AvatarLayoutDesktop]::SetCursorPos($r.R-100,$r.T+100) }
    Menu { Move-To 'Codex Badge';[AvatarLayoutDesktop]::mouse_event(8,0,0,0,[UIntPtr]::Zero);[AvatarLayoutDesktop]::mouse_event(16,0,0,0,[UIntPtr]::Zero) }
    Topmost {
        Click-Popup 'Codex Badge Menu' 80 24
    }
    Settings { Click-Popup 'Codex Badge Menu' 80 58 }
    SettingsHover { $r=@($taskWindows|Where-Object {$_.Title -eq 'Codex Badge Menu' -and $_.Visible})[0];[void][AvatarLayoutDesktop]::SetCursorPos($r.X+[int](80*$r.Dpi/96),$r.Y+[int](58*$r.Dpi/96)) }
    NotificationHover { $r=@($taskWindows|Where-Object {$_.Title -eq 'Codex Badge Menu' -and $_.Visible})[0];$x=if($SubmenuSide -eq 'Right'){264}else{80};[void][AvatarLayoutDesktop]::SetCursorPos($r.X+[int]($x*$r.Dpi/96),$r.Y+[int](92*$r.Dpi/96)) }
    CloseSettings { $r=@($taskWindows|Where-Object {$_.Title -eq 'Codex Badge Menu' -and $_.Visible})[0];[void][AvatarLayoutDesktop]::PostMessage([IntPtr]$r.Hwnd,16,[IntPtr]::Zero,[IntPtr]::Zero) }
    Startup { $x=if($SubmenuSide -eq 'Right'){264}else{80};Click-Popup 'Codex Badge Menu' $x 58 }
    DoubleClick {
        Move-To 'Codex Badge'
        for($i=0;$i -lt 2;$i++) {
            [AvatarLayoutDesktop]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
            [AvatarLayoutDesktop]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
            Start-Sleep -Milliseconds 80
        }
    }
    Notifications {
        $x=if($SubmenuSide -eq 'Right'){264}else{80};Click-Popup 'Codex Badge Menu' $x 92
    }
    CreditHover {
        $detail=@($taskWindows | Where-Object {$_.Title -eq 'Codex Badge Detail' -and $_.Visible})
        if($detail.Count -ne 1){throw 'Detail absent'}
        $button=Find-Button ([System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$detail[0].Hwnd)) '查看'
        if(!$button){throw 'Credit button absent'}
        $r=$button.Current.BoundingRectangle
        [void][AvatarLayoutDesktop]::SetCursorPos([int]($r.X+$r.Width/2),[int]($r.Y+$r.Height/2))
    }
    CreditButton {
        $detail=@($taskWindows | Where-Object {$_.Title -eq 'Codex Badge Detail' -and $_.Visible})
        if ($detail.Count -ne 1) {throw 'Detail absent'}
        $detailRoot=[System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$detail[0].Hwnd)
        Invoke-Button (Find-Button $detailRoot '查看')
    }
    Sidebar {
        $button=Find-Button $taskRoot '隐藏侧边栏'
        if (!$button) {$button=Find-Button $taskRoot '显示侧边栏'}
        Invoke-Button $button
    }
    ResizeSidebar {
        $condition=[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Separator)
        $separator=$taskRoot.FindFirst([System.Windows.Automation.TreeScope]::Descendants,$condition)
        if(!$separator){throw 'Sidebar separator absent'}
        $r=$separator.Current.BoundingRectangle
        Drag-Point ([int]($r.X+$r.Width/2)) ([int]($r.Y+$r.Height/2)) $Delta[0] 0
    }
    DragCapsule {
        $capsule=@($taskWindows | Where-Object {$_.Title -eq 'Codex Badge' -and $_.Visible})[0]
        if(!$capsule){throw 'Capsule absent'}
        Drag-Point ($capsule.X+[int]($capsule.W/2)) ($capsule.Y+[int]($capsule.H/2)) $Delta[0] $Delta[1]
    }
    RapidHover {
        $capsule=@($taskWindows | Where-Object {$_.Title -eq 'Codex Badge' -and $_.Visible})[0]
        if(!$capsule){throw 'Capsule absent'}
        for($i=0;$i -lt 12;$i++) {
            [void][AvatarLayoutDesktop]::SetCursorPos($capsule.X+[int]($capsule.W/2),$capsule.Y+[int]($capsule.H/2))
            Start-Sleep -Milliseconds 70
            [void][AvatarLayoutDesktop]::SetCursorPos($capsule.X+450,$capsule.Y-100)
            Start-Sleep -Milliseconds 70
        }
    }
    Move { $r=[AvatarLayoutDesktop+Rect]::new();[void][AvatarLayoutDesktop]::GetWindowRect($taskHwnd,[ref]$r);[void][AvatarLayoutDesktop]::SetWindowPos($taskHwnd,[IntPtr]::Zero,$r.L+50,$r.T-50,$r.R-$r.L,$r.B-$r.T,20) }
    RestoreGeometry { if($Geometry.Count -ne 4){throw 'Requires verified original geometry'};[void][AvatarLayoutDesktop]::SetWindowPos($taskHwnd,[IntPtr]::Zero,$Geometry[0],$Geometry[1],$Geometry[2],$Geometry[3],20) }
    Maximize {[void][AvatarLayoutDesktop]::ShowWindow($taskHwnd,3)}
    Restore {[void][AvatarLayoutDesktop]::ShowWindow($taskHwnd,9)}
    Minimize {[void][AvatarLayoutDesktop]::ShowWindow($taskHwnd,6)}
}
Start-Sleep -Milliseconds 700
$taskAvatar=Find-Button $taskRoot '打开个人资料菜单'
$taskSidebar=Find-Sidebar $taskRoot
$taskRect=[AvatarLayoutDesktop+Rect]::new()
[void][AvatarLayoutDesktop]::GetWindowRect($taskHwnd,[ref]$taskRect)
$taskWindows=@([AvatarLayoutDesktop]::Windows($taskBadge.Id))
if($Capture) {
    $taskCaptureRoot=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\artifacts\submenu-2026-10-01'))
    [void][IO.Directory]::CreateDirectory($taskCaptureRoot)
    $taskImagePath=Join-Path $taskCaptureRoot ([IO.Path]::GetFileName($Capture))
    $taskWidth=[Math]::Min($CaptureWidth,$taskRect.R-$taskRect.L)
    $taskHeight=[Math]::Min($CaptureHeight,$taskRect.B-$taskRect.T)
    $taskCaptureX=$taskRect.L;$taskCaptureY=$taskRect.B-$taskHeight
    if($CaptureWindow) {
        $taskCaptureTarget=@($taskWindows | Where-Object {$_.Title -eq $CaptureWindow -and $_.Visible})
        if($taskCaptureTarget.Count -ne 1){throw 'Capture window absent'}
        $taskCaptureX=$taskCaptureTarget[0].X;$taskCaptureY=$taskCaptureTarget[0].Y
        $taskWidth=$taskCaptureTarget[0].W;$taskHeight=$taskCaptureTarget[0].H
    }
    $taskBitmap=[Drawing.Bitmap]::new($taskWidth,$taskHeight)
    $taskGraphics=[Drawing.Graphics]::FromImage($taskBitmap)
    try {
        $taskGraphics.CopyFromScreen($taskCaptureX,$taskCaptureY,0,0,[Drawing.Size]::new($taskWidth,$taskHeight))
        $taskBitmap.Save($taskImagePath,[Drawing.Imaging.ImageFormat]::Png)
    } finally { $taskGraphics.Dispose();$taskBitmap.Dispose() }
}
$taskCursor=[AvatarLayoutDesktop+Cursor]::new();$taskCursor.Size=[Runtime.InteropServices.Marshal]::SizeOf($taskCursor);[void][AvatarLayoutDesktop]::GetCursorInfo([ref]$taskCursor)
[pscustomobject]@{Action=$Action;BadgePid=$taskBadge.Id;HostHwnd=$taskHwnd.ToInt64();HandCursor=($taskCursor.Handle -eq [AvatarLayoutDesktop]::LoadCursor([IntPtr]::Zero,[IntPtr]32649));
    HostRect=@($taskRect.L,$taskRect.T,($taskRect.R-$taskRect.L),($taskRect.B-$taskRect.T));
    Iconic=[AvatarLayoutDesktop]::IsIconic($taskHwnd);
    Foreground=[AvatarLayoutDesktop]::GetForegroundWindow().ToInt64();
    AvatarRect=$(if($taskAvatar){$taskAvatar.Current.BoundingRectangle.ToString()});SidebarRect=$(if($taskSidebar){$taskSidebar.Current.BoundingRectangle.ToString()});Windows=@($taskWindows | Where-Object {$_.Title -like 'Codex Badge*'})} | ConvertTo-Json -Depth 5
