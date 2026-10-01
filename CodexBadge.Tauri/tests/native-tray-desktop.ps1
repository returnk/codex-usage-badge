param([ValidateSet('Open','Observe','Dismiss','Notify')][string]$Action='Observe')
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;using System.Collections.Generic;using System.Text;using System.Runtime.InteropServices;
public static class BadgeTrayDesktop {
 public delegate bool EnumProc(IntPtr h,IntPtr p);
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int L,T,R,B; }
 [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f,IntPtr p);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h,StringBuilder s,int n);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h,StringBuilder s,int n);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);
 [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h,uint msg,IntPtr w,IntPtr l);
 [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr c);
 [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)] public struct NotifyData {
  public uint cbSize;public IntPtr hWnd;public uint uID,uFlags,uCallbackMessage;public IntPtr hIcon;
  [MarshalAs(UnmanagedType.ByValTStr,SizeConst=128)] public string szTip;
  public uint dwState,dwStateMask;
  [MarshalAs(UnmanagedType.ByValTStr,SizeConst=256)] public string szInfo;
  public uint uTimeout;
  [MarshalAs(UnmanagedType.ByValTStr,SizeConst=64)] public string szInfoTitle;
  public uint dwInfoFlags;public Guid guidItem;public IntPtr hBalloonIcon;
 }
 [DllImport("shell32.dll",CharSet=CharSet.Unicode)] public static extern bool Shell_NotifyIcon(uint command,ref NotifyData data);
 public static bool Notify(IntPtr hwnd) {var d=new NotifyData{cbSize=(uint)Marshal.SizeOf(typeof(NotifyData)),hWnd=hwnd,uID=1,uFlags=16,szTip="",szInfo="这是一条显示验收提醒，不影响真实额度。",szInfoTitle="CodexBadge 提醒显示测试",dwInfoFlags=1};return Shell_NotifyIcon(1,ref d);}
 public static object[] Windows() { var rows=new List<object>(); EnumWindows((h,p)=>{
  var title=new StringBuilder(128);var cl=new StringBuilder(128);GetWindowText(h,title,128);GetClassName(h,cl,128);
  if(title.ToString()=="CodexBadge tray"||cl.ToString()=="CodexBadgePopup") { Rect r;GetWindowRect(h,out r);uint pid;GetWindowThreadProcessId(h,out pid);
   rows.Add(new {Hwnd=h.ToInt64(),Title=title.ToString(),Class=cl.ToString(),Pid=pid,Visible=IsWindowVisible(h),X=r.L,Y=r.T,W=r.R-r.L,H=r.B-r.T}); }
  return true;},IntPtr.Zero);return rows.ToArray(); }
}
'@
[void][BadgeTrayDesktop]::SetThreadDpiAwarenessContext([IntPtr](-4))
$taskTray=@([BadgeTrayDesktop]::Windows() | Where-Object Title -eq 'CodexBadge tray')
if($taskTray.Count -ne 1){throw 'Expected exactly one badge native tray'}
if($Action -eq 'Open') {
 $taskTimer=[Diagnostics.Stopwatch]::StartNew()
 # Send the same native callback Explorer sends after tray right-button release.
 [void][BadgeTrayDesktop]::PostMessage([IntPtr]$taskTray[0].Hwnd,32809,[IntPtr]1,[IntPtr]517)
 do { $taskMenus=@([BadgeTrayDesktop]::Windows()|Where-Object {$_.Class -eq 'CodexBadgePopup' -and $_.Visible});if($taskMenus.Count){break};Start-Sleep -Milliseconds 10 }while($taskTimer.ElapsedMilliseconds -lt 3000)
 $taskTimer.Stop()
 [pscustomobject]@{NativeMenuMilliseconds=$taskTimer.ElapsedMilliseconds;Menus=$taskMenus;Tray=$taskTray} | ConvertTo-Json -Depth 4
 if($taskMenus.Count){
  $taskRect=$taskMenus[0];$taskBitmap=[Drawing.Bitmap]::new($taskRect.W,$taskRect.H);$taskGraphics=[Drawing.Graphics]::FromImage($taskBitmap)
  try{$taskGraphics.CopyFromScreen($taskRect.X,$taskRect.Y,0,0,[Drawing.Size]::new($taskRect.W,$taskRect.H));$taskBitmap.Save((Join-Path $PSScriptRoot '..\artifacts\ui-polish-2026-10-01\native-tray.png'))}finally{$taskGraphics.Dispose();$taskBitmap.Dispose()}
 }
}elseif($Action -eq 'Notify') {
 [pscustomobject]@{NotificationAccepted=[BadgeTrayDesktop]::Notify([IntPtr]$taskTray[0].Hwnd)}|ConvertTo-Json
 Start-Sleep -Milliseconds 1500
 $taskBitmap=[Drawing.Bitmap]::new(600,450);$taskGraphics=[Drawing.Graphics]::FromImage($taskBitmap)
 try{$taskGraphics.CopyFromScreen(1960,990,0,0,[Drawing.Size]::new(600,450));$taskBitmap.Save((Join-Path $PSScriptRoot '..\artifacts\ui-polish-2026-10-01\notification-test.png'))}finally{$taskGraphics.Dispose();$taskBitmap.Dispose()}
}elseif($Action -eq 'Dismiss') {
 foreach($taskMenu in @([BadgeTrayDesktop]::Windows()|Where-Object Class -eq 'CodexBadgePopup')){[void][BadgeTrayDesktop]::PostMessage([IntPtr]$taskMenu.Hwnd,16,[IntPtr]0,[IntPtr]0)}
}else{[BadgeTrayDesktop]::Windows()|ConvertTo-Json -Depth 4}
