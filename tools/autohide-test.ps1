# Issue #1: switch taskbar auto-hide on, report where the bar and the pills
# are, then put the setting back as it was. Run with BetterTaskbar running.
param([int]$Settle = 12)
$ErrorActionPreference = 'Stop'
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class AppBar {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int l, t, r, b; }
    [StructLayout(LayoutKind.Sequential)] public struct DATA {
        public int cbSize; public IntPtr hWnd; public uint uCallbackMessage; public uint uEdge; public RECT rc; public IntPtr lParam; }
    [DllImport("shell32.dll")] public static extern UIntPtr SHAppBarMessage(uint msg, ref DATA data);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    public static uint Get() { var d = new DATA(); d.cbSize = Marshal.SizeOf(d); return (uint)SHAppBarMessage(4, ref d); }
    public static void Set(uint state) { var d = new DATA(); d.cbSize = Marshal.SizeOf(d);
        d.hWnd = FindWindow("Shell_TrayWnd", null); d.lParam = (IntPtr)state; SHAppBarMessage(10, ref d); }
    public static string Bar() { RECT r; GetWindowRect(FindWindow("Shell_TrayWnd", null), out r);
        return String.Format("taskbar ({0},{1})-({2},{3})", r.l, r.t, r.r, r.b); }
}
'@
function Report($label) {
    "--- $label; auto-hide state $([AppBar]::Get()); $([AppBar]::Bar())"
    & (Join-Path $PSScriptRoot 'winstate.ps1')
}
$before = [AppBar]::Get()
Report 'before'
try {
    [AppBar]::Set(1)   # ABS_AUTOHIDE
    Start-Sleep -Seconds $Settle
    Report "auto-hide on for $Settle s"
} finally {
    [AppBar]::Set($before)
}
Start-Sleep -Seconds $Settle
Report 'restored'
