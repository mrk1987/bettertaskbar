# Issue #1: restart Explorer and report whether the docked pills come back.
# Closes open Explorer windows. Run with BetterTaskbar already running.
param([int]$Settle = 25)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$exe = Join-Path $root 'widgets\target\release\bettertaskbar-widgets.exe'
function Pills {
    Get-CimInstance Win32_Process -Filter "Name='bettertaskbar-widgets.exe'" |
        Where-Object CommandLine -match '--widget=(\S+)' |
        ForEach-Object { '{0}={1}' -f ($_.CommandLine -replace '.*--widget=(\S+).*', '$1'), $_.ProcessId } | Sort-Object
}
function Docking {
    $out = Join-Path $env:TEMP 'btw-docking.txt'
    cmd /c "`"$exe`" --check=docking > `"$out`" 2>&1"
    "exit $LASTEXITCODE"
    Get-Content $out
}
'--- before'
"pills: $((Pills) -join ' ')"
Docking
$shell = Get-Process explorer | Sort-Object StartTime | Select-Object -First 1
"--- stopping explorer pid $($shell.Id) at $(Get-Date -Format HH:mm:ss)"
Stop-Process -Id $shell.Id -Force
Start-Sleep -Seconds 5
if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) {
    'explorer did not restart on its own; starting it'
    Start-Process explorer.exe
}
Start-Sleep -Seconds $Settle
"--- after $Settle s; explorer pid $((Get-Process explorer | Sort-Object StartTime | Select-Object -First 1).Id)"
"pills: $((Pills) -join ' ')"
Docking
& (Join-Path $PSScriptRoot 'winstate.ps1')
