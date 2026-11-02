# Send key chords to one Wraith window, only while it is the foreground window:
#   keys.ps1 -ProcessId <pid> -Chords "ctrl+shift+t","alt+shift+plus"
# Aborts the moment another window has focus, so nothing is ever typed into the wrong app.
param([int]$ProcessId, [string[]]$Chords, [int]$DelayMs = 350)
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Kb {
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  [DllImport("user32.dll")] public static extern uint MapVirtualKey(uint code, uint mapType);
}
'@
$vk = @{ ctrl=0x11; shift=0x10; alt=0x12; tab=0x09; enter=0x0D; esc=0x1B; left=0x25; up=0x26; right=0x27; down=0x28;
         plus=0xBB; minus=0xBD; comma=0xBC; period=0xBE; space=0x20; backspace=0x08 }
foreach ($c in [char[]]'abcdefghijklmnopqrstuvwxyz') { $vk["$c"] = [int][char]::ToUpper($c) }
foreach ($d in 0..9) { $vk["$d"] = 0x30 + $d }
$h = (Get-Process -Id $ProcessId).MainWindowHandle
[Kb]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 300
foreach ($chord in $Chords) {
  if ([Kb]::GetForegroundWindow() -ne $h) { throw "Wraith lost the foreground; stopped before sending '$chord'" }
  $keys = $chord.ToLower().Split('+') | ForEach-Object { $vk[$_] }
  # Real scan codes: Wraith identifies keys by scan code, so a zero scan code is "unidentified".
  $ext = @(0x25, 0x26, 0x27, 0x28)
  foreach ($k in $keys) { [Kb]::keybd_event([byte]$k, [byte][Kb]::MapVirtualKey($k, 0), $(if ($ext -contains $k) { 1 } else { 0 }), [UIntPtr]::Zero) }
  [array]::Reverse($keys)
  foreach ($k in $keys) { [Kb]::keybd_event([byte]$k, [byte][Kb]::MapVirtualKey($k, 0), 2 + $(if ($ext -contains $k) { 1 } else { 0 }), [UIntPtr]::Zero) }
  Start-Sleep -Milliseconds $DelayMs
}
