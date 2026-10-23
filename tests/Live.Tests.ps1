# Live apply/undo check on THIS PC. Changes real settings, so it refuses to run without -IUnderstand.
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\Live.Tests.ps1 -IUnderstand [-Only id1,id2]
# Rules: administrator only; a restore point first (stops if it fails); only tweaks that are NOT active now are
# tested, so the current setup is never undone; uninstalls without an undo and security/update/encryption changes
# are skipped. For each tweak: footprint, apply, verify, undo, footprint again; any difference is a failure.
param([switch]$IUnderstand, [string[]]$Only, [string]$Report = (Join-Path $env:TEMP 'optimaxer-live-report.json'))

$ErrorActionPreference = 'Continue'
$Only = @($Only | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
if (-not $IUnderstand) { Write-Host 'This changes real settings. Run again with -IUnderstand.'; exit 2 }
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) { Write-Host 'Run as administrator.'; exit 2 }

$global:OptiRoot = Split-Path $PSScriptRoot -Parent
. "$global:OptiRoot\src\Core.ps1"
. "$global:OptiRoot\src\Data.ps1"
. "$global:OptiRoot\src\Catalog.ps1"
$global:OptiConsole = $false

# never run live: no undo (uninstalls), or touches encryption, updates, remote access or the clock
$skip = @{
    'sto-bitlocker'     = 'turns off BitLocker encryption (decrypts drives)'
    'wu-security'       = 'changes Windows Update'
    'wu-pause'          = 'pauses Windows Update'
    'net-rdp-unsigned'  = 'lowers Remote Desktop security'
    'sys-utc'           = 'changes how the hardware clock is read'
    'perf-vbs'          = 'lowers kernel security'
    'debloat-onedrive'  = 'uninstall without undo'
    'debloat-edge'      = 'uninstall without undo'
    'ai-remove-apps'    = 'uninstall without undo'
}

function Get-Footprint {
    # Everything a tweak can touch that can be read back: registry values, service start types, task states,
    # its own Test, and for script tweaks the values their snapshot reads.
    param($T)
    $f = [ordered]@{}
    foreach ($r in $T.Reg) { $s = Get-RegSnapshot $r; $f["reg $($r.P)\$($r.N)"] = $(if ($s.Exists) { "$($s.Type):$(@($s.Value) -join ',')" } else { '<none>' }) }
    foreach ($s in $T.Svc) { $x = Get-SvcSnapshot $s.Name; if ($x) { $f["svc $($s.Name)"] = "$($x.Start)/$($x.Delayed)" } }
    Clear-TaskCache
    foreach ($tk in $T.Task) { $x = Get-TaskSnapshot $tk; if ($x) { $f["task $tk"] = $x.Enabled } }
    if ($T.Snapshot) { try { $f['snapshot'] = (& $T.Snapshot | ConvertTo-Json -Depth 6 -Compress) } catch { $f['snapshot'] = "error: $($_.Exception.Message)" } }
    $f['test'] = [bool](Test-Tweak $T)
    return $f
}

Write-Host 'Reading which tweaks are active...'
$status = @{}; foreach ($s in Get-TweakStatus) { $status[$s.Id] = $s.On }
$todo = @($global:OptiCatalog | Where-Object { -not $status[$_.Id] -and -not $skip.ContainsKey($_.Id) -and (-not $Only -or $Only -contains $_.Id) })
Write-Host "$($todo.Count) inactive tweak(s) to test; $(@($status.Values | Where-Object { $_ }).Count) active ones are left alone."

if (-not (New-OptiRestorePoint 'Optimaxer - before live apply/undo test')) { Write-Host 'Restore point failed: stopping, nothing was changed.'; exit 3 }

$results = @()
foreach ($t in $todo) {
    $r = [ordered]@{ Id = $t.Id; Name = $t.Name; Risk = $t.Risk; Restart = $t.Restart; Applied = $false; UndoClean = $false; Diff = @(); Error = '' }
    try {
        $before = Get-Footprint $t
        $r.Applied = [bool](Invoke-TweakApply $t)
        $global:OptiUndoFailed = @{}
        [void](Invoke-TweakUndo $t)
        if ($global:OptiUndoFailed.ContainsKey($t.Id)) { $r.Error = 'undo reported: ' + ($global:OptiUndoFailed[$t.Id] -join '; ') }
        $after = Get-Footprint $t
        foreach ($k in $before.Keys) { if ("$($before[$k])" -ne "$($after[$k])") { $r.Diff += "$k : $($before[$k]) -> $($after[$k])" } }
        $r.UndoClean = (-not $r.Diff.Count) -and -not $r.Error
    } catch { $r.Error = $_.Exception.Message }
    $mark = if ($r.Applied -and $r.UndoClean) { 'ok  ' } elseif ($r.UndoClean) { 'NOAP' } else { 'FAIL' }
    Write-Host ("{0} {1,-22} applied={2} undo-clean={3} {4}" -f $mark, $t.Id, $r.Applied, $r.UndoClean, $(if ($r.Error) { $r.Error } elseif ($r.Diff) { $r.Diff -join ' | ' } else { '' }))
    $results += [pscustomobject]$r
}
$results | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $Report -Encoding UTF8
$bad = @($results | Where-Object { -not $_.UndoClean })
$noap = @($results | Where-Object { $_.UndoClean -and -not $_.Applied })
Write-Host ''
Write-Host ("{0} tested: {1} applied and undone cleanly, {2} did not verify after apply, {3} left something changed. Report: {4}" -f $results.Count, @($results | Where-Object { $_.Applied -and $_.UndoClean }).Count, $noap.Count, $bad.Count, $Report)
Write-Host ("Skipped on purpose: " + (($skip.Keys | Sort-Object) -join ', '))
if ($bad.Count) { exit 1 }
