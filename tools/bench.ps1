# Launch the release build hidden, then sample per-process CPU (% of one core),
# private working set (actual RAM) and private committed memory over a window.
# Stop the processes afterwards yourself (by Id, not by name).
param([int]$Warmup = 20, [int]$Window = 40)
$ErrorActionPreference = 'Stop'
$exe = Join-Path $PSScriptRoot '..\widgets\target\release\bettertaskbar-widgets.exe'
Start-Process -FilePath $exe -WindowStyle Hidden
Start-Sleep -Seconds $Warmup
function Snap {
    Get-Process bettertaskbar-widgets -ErrorAction SilentlyContinue | ForEach-Object {
        $cmd = (Get-CimInstance Win32_Process -Filter "ProcessId=$($_.Id)").CommandLine
        [pscustomobject]@{ Id = $_.Id; Cpu = $_.TotalProcessorTime.TotalSeconds; Commit = $_.PrivateMemorySize64; Cmd = $cmd }
    }
}
$a = Snap
$t0 = Get-Date
Start-Sleep -Seconds $Window
$b = Snap
$el = ((Get-Date) - $t0).TotalSeconds
$pws = @{}
Get-CimInstance Win32_PerfFormattedData_PerfProc_Process | Where-Object Name -like 'bettertaskbar*' | ForEach-Object { $pws[[int]$_.IDProcess] = [int64]$_.WorkingSetPrivate }
$rows = foreach ($p in $b) {
    $prev = $a | Where-Object Id -eq $p.Id
    $role = if ($p.Cmd -match '--widget[= ](\S+)') { $Matches[1] } elseif ($p.Cmd -match '--(\w+)') { $Matches[1] } else { 'hub' }
    [pscustomobject]@{
        Id = $p.Id; Role = $role
        CpuPctOneCore = if ($prev) { [math]::Round(100 * ($p.Cpu - $prev.Cpu) / $el, 2) } else { $null }
        PrivWS_MiB = [math]::Round($pws[$p.Id] / 1MB, 1)
        Commit_MiB = [math]::Round($p.Commit / 1MB, 1)
    }
}
$rows | Format-Table -AutoSize | Out-String -Width 200
"Window: {0:N1}s  Processes: {1}" -f $el, $rows.Count
"Totals: CPU {0:N2}% of one core, private WS {1:N1} MiB, committed {2:N1} MiB" -f ($rows | Measure-Object CpuPctOneCore -Sum).Sum, ($rows | Measure-Object PrivWS_MiB -Sum).Sum, ($rows | Measure-Object Commit_MiB -Sum).Sum
