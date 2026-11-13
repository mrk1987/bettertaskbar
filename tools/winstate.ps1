# Print every BetterTaskbar window's live Win32 state: rect, visibility,
# child/popup style, topmost, cloaked and parent. A pill that is a child of the
# taskbar but outside its rect is clipped invisible (see commit 35ea246).
Add-Type @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc cb, IntPtr l);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint f);
  [DllImport("user32.dll", EntryPoint="GetWindowLongPtrW")] public static extern IntPtr GetLong(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out int v, int s);
  public static Dictionary<string, IntPtr> Found = new Dictionary<string, IntPtr>();
  static string Title(IntPtr h) { var sb = new StringBuilder(256); GetWindowTextW(h, sb, 256); return sb.ToString(); }
  static bool Child(IntPtr h, IntPtr l) { var t = Title(h); if (t.StartsWith("BetterTaskbar ")) Found[t] = h; return true; }
  static bool Top(IntPtr h, IntPtr l) { Child(h, l); EnumChildWindows(h, Child, IntPtr.Zero); return true; }
  public static void Scan() { Found.Clear(); EnumWindows(Top, IntPtr.Zero); }
}
"@
[W]::Scan()
foreach ($kv in [W]::Found.GetEnumerator() | Sort-Object Key) {
  $h = $kv.Value; $n = $kv.Key -replace '^BetterTaskbar ', ''
  $r = New-Object W+RECT; [void][W]::GetWindowRect($h, [ref]$r)
  $style = [W]::GetLong($h, -16).ToInt64(); $ex = [W]::GetLong($h, -20).ToInt64()
  $cloak = 0; [void][W]::DwmGetWindowAttribute($h, 14, [ref]$cloak, 4)
  $parent = [W]::GetAncestor($h, 1)
  "{0,-12} ({1},{2})-({3},{4}) WS_VISIBLE={5} IsVisible={6} child={7} popup={8} topmost={9} cloaked={10} parent=0x{11:X}" -f $n, $r.L, $r.T, $r.R, $r.B, (($style -band 0x10000000) -ne 0), [W]::IsWindowVisible($h), (($style -band 0x40000000) -ne 0), (($style -band 0x80000000) -ne 0), (($ex -band 0x8) -ne 0), $cloak, $parent.ToInt64()
}
