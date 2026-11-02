# Mouse input for one Wraith window, in client pixels, only while it is the foreground window:
#   mouse.ps1 -ProcessId <pid> -Actions "click:100,10","drag:500,300,250,300","rightclick:10,10"
param([int]$ProcessId, [string[]]$Actions, [int]$DelayMs = 350)
Add-Type @'
using System;
using System.Runtime.InteropServices;
public struct PT { public int X, Y; }
public static class Ms {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref PT p);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out PT p);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(PT p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint flags);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, int data, UIntPtr extra);
}
'@
$h = (Get-Process -Id $ProcessId).MainWindowHandle
[Ms]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 300
function MoveTo($x, $y) {
  $p = New-Object PT; $p.X = $x; $p.Y = $y
  [Ms]::ClientToScreen($h, [ref]$p) | Out-Null
  [Ms]::SetCursorPos($p.X, $p.Y) | Out-Null
  Start-Sleep -Milliseconds 60
}
# Never click unless the pointer is over this very window.
function Guard($what) {
  if ([Ms]::GetForegroundWindow() -ne $h) { throw "Wraith lost the foreground; stopped before '$what'" }
}
function OverWindow() {
  $p = New-Object PT; [Ms]::GetCursorPos([ref]$p) | Out-Null
  [Ms]::GetAncestor([Ms]::WindowFromPoint($p), 2) -eq $h
}
function Press($flags) { if (-not (OverWindow)) { throw 'pointer is not over Wraith; no click sent' }; [Ms]::mouse_event($flags, 0, 0, 0, [UIntPtr]::Zero) }
foreach ($a in $Actions) {
  Guard $a
  $kind, $args_ = $a.Split(':', 2)
  $n = $args_.Split(',') | ForEach-Object { [int]$_ }
  switch ($kind) {
    'click'      { MoveTo $n[0] $n[1]; Press 0x02; Press 0x04 }
    'rightclick' { MoveTo $n[0] $n[1]; Press 0x08; Press 0x10 }
    'move'       { MoveTo $n[0] $n[1] }
    'drag'       {
      MoveTo $n[0] $n[1]; Press 0x02
      foreach ($i in 1..8) { MoveTo ([int]($n[0] + ($n[2] - $n[0]) * $i / 8)) ([int]($n[1] + ($n[3] - $n[1]) * $i / 8)) }
      Press 0x04
    }
  }
  Start-Sleep -Milliseconds $DelayMs
}
