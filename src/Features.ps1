# Tools that change settings outside the tweak catalog: boot cost, per-game profiles, Search exclusions, wake sources,
# Appx checks and measurements (frame times, latency, boot). Each change saves what was there first (features.json) and has a Test and
# an Undo; Undo ALL in Tools calls Undo-OptiFeatures. Dot-sourced by Core.ps1.

$global:OptiFeaturesPath = Join-Path $global:OptiData 'features.json'
$global:OptiMeasurePath = Join-Path $global:OptiData 'measurements.json'
$global:OptiToolsDir = Join-Path $global:OptiData 'tools'

function Get-OptiFeatures {
    $h = @{ GameProfiles = @{}; SearchExclusions = @{}; Wake = @{ Devices = @(); Tasks = @() } }
    if (Test-Path -LiteralPath $global:OptiFeaturesPath) {
        try {
            $o = Get-Content -LiteralPath $global:OptiFeaturesPath -Raw -ErrorAction Stop | ConvertFrom-Json
            if ($o.GameProfiles) { foreach ($p in $o.GameProfiles.PSObject.Properties) { $h.GameProfiles[$p.Name] = $p.Value } }
            if ($o.SearchExclusions) { foreach ($p in $o.SearchExclusions.PSObject.Properties) { $h.SearchExclusions[$p.Name] = $p.Value } }
            if ($o.Wake) { $h.Wake = @{ Devices = @($o.Wake.Devices | Where-Object { $_ }); Tasks = @($o.Wake.Tasks | Where-Object { $_ }) } }
        } catch { Write-Log "features.json unreadable: $($_.Exception.Message)" 'WARN' }
    }
    return $h
}

function Save-OptiFeatures {
    param($F)
    $tmp = "$global:OptiFeaturesPath.tmp"
    $F | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $tmp -Encoding UTF8 -ErrorAction Stop
    Move-Item -LiteralPath $tmp -Destination $global:OptiFeaturesPath -Force -ErrorAction Stop
}

# ---------------------------------------------------------------- boot cost (read-only; disabling uses the Startup page)
function Get-EventData {
    # Named <Data> fields of one event as a hashtable.
    param($Event)
    $h = @{}
    try { foreach ($d in ([xml]$Event.ToXml()).Event.EventData.Data) { $h[$d.Name] = $d.'#text' } } catch {}
    return $h
}

function Get-BootCost {
    # Reads Diagnostics-Performance: the last boots (event 100) and the apps, services and drivers Windows itself
    # flagged as slowing boot (101, 103, 109). Each slow item is matched to a startup entry when one runs that file.
    param([int]$Max = 15)
    $log = 'Microsoft-Windows-Diagnostics-Performance/Operational'
    $boots = @(); $slow = @()
    try { $ev = @(Get-WinEvent -FilterHashtable @{ LogName = $log; Id = 100, 101, 103, 109 } -MaxEvents 400 -ErrorAction Stop) }
    catch { return @{ Error = "Cannot read $log ($($_.Exception.Message)). It needs administrator rights and may be empty on a new install."; Boots = @(); Slow = @() } }
    $startup = @(Get-StartupItems)
    foreach ($e in $ev) {
        $d = Get-EventData $e
        if ($e.Id -eq 100) { $boots += [pscustomobject]@{ Time = $e.TimeCreated; BootMs = [int]$d['BootTime']; MainPathMs = [int]$d['MainPathBootTime']; PostBootMs = [int]$d['BootPostBootTime'] }; continue }
        $file = [string]$d['FileName']; if (-not $file) { $file = [string]$d['Name'] }
        $kind = switch ($e.Id) { 101 { 'app' } 103 { 'service' } default { 'driver' } }
        $match = $null
        $leaf = if ($file) { [IO.Path]::GetFileName($file) } else { '' }
        if ($leaf) { $match = $startup | Where-Object { $_.Cmd -and $_.Cmd -like "*$leaf*" } | Select-Object -First 1 }
        $slow += [pscustomobject]@{
            Name = $(if ($d['FriendlyName']) { $d['FriendlyName'] } else { $leaf }); File = $file; Kind = $kind
            Ms = [int]$d['DegradationTime']; TotalMs = [int]$d['TotalTime']; Time = $e.TimeCreated
            Startup = $(if ($match) { $match.Name } else { '' }); Item = $match
        }
    }
    # one line per file: the worst delay Windows recorded for it
    $worst = @($slow | Group-Object { "$($_.Kind)|$($_.File)" } | ForEach-Object { $_.Group | Sort-Object Ms -Descending | Select-Object -First 1 } | Sort-Object Ms -Descending | Select-Object -First $Max)
    return @{ Error = ''; Boots = @($boots | Sort-Object Time -Descending | Select-Object -First 5); Slow = $worst }
}

# ---------------------------------------------------------------- per-game profile
$global:OptiGpuPrefKey = 'HKCU:\Software\Microsoft\DirectX\UserGpuPreferences'
$global:OptiLayersKey = 'HKCU:\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers'
$global:OptiFsoFlag = 'DISABLEDXMAXIMIZEDWINDOWEDMODE'

function Set-KeyValuePair {
    # 'A=1;B=2;' with Key set to Value, other pairs kept in order.
    param([string]$Text, [string]$Key, [string]$Value)
    $pairs = [ordered]@{}
    foreach ($p in ($Text -split ';')) { if ($p -match '^\s*([^=]+?)\s*=\s*(.*)$') { $pairs[$Matches[1]] = $Matches[2] } }
    $pairs[$Key] = $Value
    return ((@($pairs.Keys | ForEach-Object { "$_=$($pairs[$_])" }) -join ';') + ';')
}

function Get-ValueSnapshot {
    param([string]$Path, [string]$Name)
    $s = Get-RegSnapshot @{ P = $Path; N = $Name }
    return @{ Exists = [bool]$s.Exists; Value = $s.Value; Type = $s.Type }
}

function Add-GameProfile {
    # For each exe: high-performance GPU and fullscreen optimizations off, for that exe only. The previous values are
    # saved first; a second run on the same exe keeps the first saved values.
    param([string[]]$Exe)
    $f = Get-OptiFeatures
    foreach ($x in $Exe) {
        if (-not $x -or -not (Test-Path -LiteralPath $x)) { Write-Log "  not found: $x" 'WARN'; continue }
        if (-not $f.GameProfiles.ContainsKey($x)) {
            $f.GameProfiles[$x] = @{ Gpu = (Get-ValueSnapshot $global:OptiGpuPrefKey $x); Layers = (Get-ValueSnapshot $global:OptiLayersKey $x) }
            Save-OptiFeatures $f
        }
        $gpu = Get-ValueSnapshot $global:OptiGpuPrefKey $x
        Set-RegEntry $global:OptiGpuPrefKey $x (Set-KeyValuePair $(if ($gpu.Exists) { [string]$gpu.Value } else { '' }) 'GpuPreference' '2') 'String'
        $lay = Get-ValueSnapshot $global:OptiLayersKey $x
        $cur = if ($lay.Exists) { [string]$lay.Value } else { '' }
        if ($cur -notmatch "\b$global:OptiFsoFlag\b") {
            $new = if ($cur.Trim()) { "$($cur.Trim()) $global:OptiFsoFlag" } else { "~ $global:OptiFsoFlag" }
            Set-RegEntry $global:OptiLayersKey $x $new 'String'
        }
        Write-Log "  profile set: $x (high-performance GPU, fullscreen optimizations off)" 'OK'
    }
}

function Test-GameProfile {
    param([string]$Exe)
    $gpu = Get-ValueSnapshot $global:OptiGpuPrefKey $Exe
    $lay = Get-ValueSnapshot $global:OptiLayersKey $Exe
    return ($gpu.Exists -and [string]$gpu.Value -match 'GpuPreference=2;' -and $lay.Exists -and [string]$lay.Value -match "\b$global:OptiFsoFlag\b")
}

function Remove-GameProfile {
    # Puts back exactly what was there: a value this tool created is deleted, a value it changed gets its old text.
    param([string[]]$Exe)
    $f = Get-OptiFeatures
    $bad = @()
    foreach ($x in @($(if ($Exe) { $Exe } else { @($f.GameProfiles.Keys) }))) {
        $rec = $f.GameProfiles[$x]
        if (-not $rec) { continue }
        $ok = (Restore-RegSnapshot @{ P = $global:OptiGpuPrefKey; N = $x; Exists = [bool]$rec.Gpu.Exists; Value = $rec.Gpu.Value; Type = $(if ($rec.Gpu.Type) { $rec.Gpu.Type } else { 'String' }) }) -and
              (Restore-RegSnapshot @{ P = $global:OptiLayersKey; N = $x; Exists = [bool]$rec.Layers.Exists; Value = $rec.Layers.Value; Type = $(if ($rec.Layers.Type) { $rec.Layers.Type } else { 'String' }) })
        if ($ok) { $f.GameProfiles.Remove($x); Write-Log "  profile removed: $x" 'OK' } else { $bad += $x }
    }
    Save-OptiFeatures $f
    return $bad.Count -eq 0
}

# ---------------------------------------------------------------- Windows Search exclusions (the indexer stays on)
function Test-SearchRunning {
    # Exclusions live in the indexer's crawl scope; with Windows Search stopped there is nothing to change.
    return ((Get-Service -Name WSearch -ErrorAction SilentlyContinue).Status -eq 'Running')
}

function Initialize-OptiSearch {
    # Compiled on first use only: most jobs never touch Search, and Add-Type costs about a second.
    if ('OptiSearch.Scope' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
namespace OptiSearch {
    // Vtable order from searchapi.h. Methods this tool never calls are placeholders that only keep the slots aligned.
    [ComImport, Guid("AB310581-AC80-11D1-8DF3-00C04FB6EF69"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface ISearchManager {
        void _GetIndexerVersionStr(); void _GetIndexerVersion(); void _GetParameter(); void _SetParameter();
        void _ProxyName(); void _BypassList(); void _SetProxy();
        [return: MarshalAs(UnmanagedType.Interface)] ISearchCatalogManager GetCatalog([MarshalAs(UnmanagedType.LPWStr)] string name);
    }
    [ComImport, Guid("AB310581-AC80-11D1-8DF3-00C04FB6EF50"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface ISearchCatalogManager {
        void _Name(); void _GetParameter(); void _SetParameter(); void _GetCatalogStatus(); void _Reset(); void _Reindex();
        void _ReindexMatchingURLs(); void _ReindexSearchRoot(); void _putConnectTimeout(); void _getConnectTimeout();
        void _putDataTimeout(); void _getDataTimeout(); void _NumberOfItems(); void _NumberOfItemsToIndex(); void _URLBeingIndexed();
        void _GetURLIndexingState(); void _GetPersistentItemsChangedSink(); void _RegisterViewForNotification();
        void _GetItemsChangedSink(); void _UnregisterViewForNotification(); void _SetExtensionClusion(); void _EnumerateExcludedExtensions();
        void _GetQueryHelper(); void _putDiacriticSensitivity(); void _getDiacriticSensitivity();
        [return: MarshalAs(UnmanagedType.Interface)] ISearchCrawlScopeManager GetCrawlScopeManager();
    }
    [ComImport, Guid("AB310581-AC80-11D1-8DF3-00C04FB6EF55"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface ISearchCrawlScopeManager {
        void _AddDefaultScopeRule(); void _AddRoot(); void _RemoveRoot(); void _EnumerateRoots(); void _AddHierarchicalScope();
        void AddUserScopeRule([MarshalAs(UnmanagedType.LPWStr)] string url, int include, int overrideChildren, uint followFlags);
        void RemoveScopeRule([MarshalAs(UnmanagedType.LPWStr)] string url);
        void _EnumerateScopeRules(); void _HasParentScopeRule(); void _HasChildScopeRule();
        int IncludedInCrawlScope([MarshalAs(UnmanagedType.LPWStr)] string url);
        void _IncludedInCrawlScopeEx(); void _RevertToDefaultScopes();
        void SaveAll();
    }
    public static class Scope {
        static ISearchCrawlScopeManager Csm() {
            var t = Type.GetTypeFromCLSID(new Guid("7D096C5F-AC08-4F1F-BEB7-5C22C517CE39"));
            var sm = (ISearchManager)Activator.CreateInstance(t);
            return sm.GetCatalog("SystemIndex").GetCrawlScopeManager();
        }
        public static bool Included(string url) { return Csm().IncludedInCrawlScope(url) != 0; }
        public static void Exclude(string[] urls) { var c = Csm(); foreach (var u in urls) c.AddUserScopeRule(u, 0, 1, 1); c.SaveAll(); }
        public static void Remove(string[] urls) { var c = Csm(); foreach (var u in urls) c.RemoveScopeRule(u); c.SaveAll(); }
    }
}
'@
}

$global:OptiDevTreeNames = @('node_modules', '.git', 'target', 'build')

function ConvertTo-SearchUrl { param([string]$Dir) return 'file:///' + $Dir.TrimEnd('\') + '\' }

function Find-DevTrees {
    # Build and dependency folders under the profile. Does not descend into a hit, AppData or links. Capped.
    param([string]$Root = $env:USERPROFILE, [int]$MaxDepth = 6, [int]$Max = 400)
    $hits = New-Object System.Collections.ArrayList
    $stack = New-Object System.Collections.Stack
    $stack.Push(@{ Dir = (New-Object IO.DirectoryInfo $Root); Depth = 0 })
    while ($stack.Count -and $hits.Count -lt $Max) {
        $n = $stack.Pop()
        try { $subs = $n.Dir.GetDirectories() } catch { continue }
        foreach ($s in $subs) {
            if ($s.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
            if ($n.Depth -eq 0 -and $s.Name -eq 'AppData') { continue }
            if ($global:OptiDevTreeNames -contains $s.Name) { [void]$hits.Add($s.FullName); continue }
            if ($n.Depth + 1 -lt $MaxDepth) { $stack.Push(@{ Dir = $s; Depth = $n.Depth + 1 }) }
        }
    }
    return @($hits | Sort-Object)
}

function Add-SearchExclusions {
    # Saves whether each folder was indexed, then adds an exclude rule only for folders that were.
    param([string[]]$Dirs)
    if (-not (Test-SearchRunning)) { Write-Log '  Windows Search is not running on this PC, so nothing is indexed and nothing needs excluding.' 'WARN'; return 0 }
    Initialize-OptiSearch
    $f = Get-OptiFeatures
    $add = @()
    foreach ($d in $Dirs) {
        $u = ConvertTo-SearchUrl $d
        if ($f.SearchExclusions.ContainsKey($u)) { continue }
        $was = [OptiSearch.Scope]::Included($u)
        if (-not $was) { Write-Log "  already not indexed: $d"; continue }
        $f.SearchExclusions[$u] = @{ Dir = $d; WasIncluded = $true; Time = (Get-Date).ToString('s') }
        $add += $u
    }
    Save-OptiFeatures $f
    if ($add.Count) { [OptiSearch.Scope]::Exclude($add); Write-Log "  $($add.Count) folder(s) excluded from Windows Search; the indexer itself stays on." 'OK' }
    return $add.Count
}

function Test-SearchExclusions {
    $f = Get-OptiFeatures
    if (-not $f.SearchExclusions.Count -or -not (Test-SearchRunning)) { return $true }
    Initialize-OptiSearch
    foreach ($u in $f.SearchExclusions.Keys) { if ([OptiSearch.Scope]::Included($u)) { return $false } }
    return $true
}

function Remove-SearchExclusions {
    $f = Get-OptiFeatures
    $urls = @($f.SearchExclusions.Keys)
    if (-not $urls.Count) { return $true }
    if (-not (Test-SearchRunning)) { Write-Log '  Windows Search is not running; start it, then remove the exclusions again.' 'WARN'; return $false }
    Initialize-OptiSearch
    try { [OptiSearch.Scope]::Remove($urls) } catch { Write-Log "  could not remove the Search rules: $($_.Exception.Message)" 'WARN'; return $false }
    $f.SearchExclusions = @{}
    Save-OptiFeatures $f
    Write-Log "  $($urls.Count) Search exclusion(s) removed." 'OK'
    return $true
}

# ---------------------------------------------------------------- wake sources
function Test-ProtectedWakeTask {
    # Update and Defender tasks are never offered: they wake the PC on purpose.
    param([string]$Path)
    return ($Path -like '\Microsoft\Windows\UpdateOrchestrator\*' -or $Path -like '\Microsoft\Windows\WindowsUpdate\*' -or $Path -like '*Windows Defender*' -or $Path -like '*\Microsoft\Windows\Defender*')
}

function Get-WakeSources {
    $devices = @((Invoke-Native powercfg.exe @('/devicequery', 'wake_armed')) -split "[\r\n]+" | ForEach-Object { $_.Trim() } | Where-Object { $_ -and $_ -ne 'NONE' })
    $tasks = @()
    foreach ($t in @(Get-ScheduledTask -ErrorAction SilentlyContinue)) {
        if (-not $t.Settings.WakeToRun -or $t.State -eq 'Disabled') { continue }
        if (Test-ProtectedWakeTask $t.TaskPath) { continue }
        $tasks += [pscustomobject]@{ Path = $t.TaskPath; Name = $t.TaskName; Full = $t.TaskPath + $t.TaskName }
    }
    return @{ Devices = $devices; Tasks = $tasks }
}

function Disable-WakeSources {
    # Only what the user ticked. Each one is recorded before it is changed.
    param([string[]]$Devices, [string[]]$Tasks)
    $f = Get-OptiFeatures
    foreach ($d in $Devices) {
        if ($f.Wake.Devices -notcontains $d) { $f.Wake.Devices += $d; Save-OptiFeatures $f }
        $o = Invoke-Native powercfg.exe @('/devicedisablewake', $d)
        if ($global:OptiExit -ne 0) { Write-Log "  $d : $($o.Trim())" 'WARN'; $f.Wake.Devices = @($f.Wake.Devices | Where-Object { $_ -ne $d }); Save-OptiFeatures $f }
        else { Write-Log "  can no longer wake the PC: $d" 'OK' }
    }
    foreach ($full in $Tasks) {
        $s = Split-TaskPath $full
        if (Test-ProtectedWakeTask $s.Path) { Write-Log "  left alone (update or Defender task): $full" 'WARN'; continue }
        if ($f.Wake.Tasks -notcontains $full) { $f.Wake.Tasks += $full; Save-OptiFeatures $f }
        try {
            $t = Get-ScheduledTask -TaskPath $s.Path -TaskName $s.Name -ErrorAction Stop
            $t.Settings.WakeToRun = $false
            Set-ScheduledTask -TaskPath $s.Path -TaskName $s.Name -Settings $t.Settings -ErrorAction Stop | Out-Null
            Write-Log "  task no longer wakes the PC: $full" 'OK'
        } catch {
            Write-Log "  $full : $($_.Exception.Message)" 'WARN'
            $f.Wake.Tasks = @($f.Wake.Tasks | Where-Object { $_ -ne $full }); Save-OptiFeatures $f
        }
    }
}

function Test-WakeSourcesDisabled {
    $f = Get-OptiFeatures
    $now = Get-WakeSources
    foreach ($d in $f.Wake.Devices) { if ($now.Devices -contains $d) { return $false } }
    foreach ($t in $f.Wake.Tasks) { if (@($now.Tasks | ForEach-Object Full) -contains $t) { return $false } }
    return $true
}

function Restore-WakeSources {
    $f = Get-OptiFeatures
    $leftDev = @(); $leftTask = @()
    foreach ($d in $f.Wake.Devices) {
        $null = Invoke-Native powercfg.exe @('/deviceenablewake', $d)
        if ($global:OptiExit -ne 0) { $leftDev += $d } else { Write-Log "  can wake the PC again: $d" 'OK' }
    }
    foreach ($full in $f.Wake.Tasks) {
        $s = Split-TaskPath $full
        try {
            $t = Get-ScheduledTask -TaskPath $s.Path -TaskName $s.Name -ErrorAction Stop
            $t.Settings.WakeToRun = $true
            Set-ScheduledTask -TaskPath $s.Path -TaskName $s.Name -Settings $t.Settings -ErrorAction Stop | Out-Null
        } catch { $leftTask += $full }
    }
    $f.Wake = @{ Devices = $leftDev; Tasks = $leftTask }
    Save-OptiFeatures $f
    if ($leftDev.Count -or $leftTask.Count) { Write-Log "  could not restore: $(@($leftDev + $leftTask) -join ', ')" 'WARN'; return $false }
    return $true
}

# ---------------------------------------------------------------- Appx: installed AND provisioned must be gone
function Test-AppxGone {
    # False while any matching package is still installed for a user or still provisioned for new accounts.
    param([string[]]$Pattern)
    foreach ($pat in $Pattern) {
        foreach ($p in @(Get-AppxPackage -AllUsers -Name $pat -ErrorAction SilentlyContinue)) { if (-not $p.NonRemovable -and -not $p.IsFramework) { return $false } }
        try { if (@(Get-AppxProvisionedPackage -Online -ErrorAction Stop | Where-Object { $_.DisplayName -like $pat }).Count) { return $false } } catch {}
    }
    return $true
}

# ---------------------------------------------------------------- measurements (Tools > Measure)
# Real numbers from this PC, saved as labelled runs so before/after can be compared:
#   frames  : Intel PresentMon (MIT, fetched from its GitHub release on first use, checked against the published
#             SHA-256) records every frame of a game you name: average FPS, 1% and 0.1% lows, 99th percentile
#             frame time, stutters.
#   latency : timer resolution, DPC and interrupt CPU time, ping and jitter to 1.1.1.1.
#   boot    : the last boot time Windows logged.

function Get-PresentMonPath {
    $exe = Join-Path $global:OptiToolsDir 'PresentMon.exe'
    if (Test-Path -LiteralPath $exe) { return $exe }
    New-Item -ItemType Directory -Path $global:OptiToolsDir -Force | Out-Null
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $r = Invoke-RestMethod -Uri 'https://api.github.com/repos/GameTechDev/PresentMon/releases/latest' -Headers @{ 'User-Agent' = 'Optimaxer' } -TimeoutSec 20 -ErrorAction Stop
    $asset = @($r.assets | Where-Object { $_.name -like 'PresentMon-*-x64.exe' })[0]
    if (-not $asset) { throw 'the PresentMon release has no x64 exe' }
    if ([string]$asset.digest -notmatch '^sha256:(?<h>[0-9a-fA-F]{64})$') { throw 'the PresentMon release has no published checksum' }
    $want = $Matches['h']
    $tmp = "$exe.download"
    Write-Log "Downloading $($asset.name) from GitHub (Intel PresentMon, MIT licence)..."
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmp -UseBasicParsing -TimeoutSec 120 -ErrorAction Stop
    $got = (Get-FileHash -LiteralPath $tmp -Algorithm SHA256).Hash
    if ($got -ne $want) { Remove-Item -LiteralPath $tmp -Force; throw "PresentMon rejected: checksum mismatch (expected $want, got $got)" }
    Move-Item -LiteralPath $tmp -Destination $exe -Force
    Write-Log 'PresentMon verified (SHA-256).' 'OK'
    return $exe
}

function Get-FrameStats {
    # Frame times in ms -> the numbers reviewers use. Lows are the average FPS of the slowest 1% / 0.1% of frames.
    param([double[]]$FrameTimes)
    $ft = @($FrameTimes | Where-Object { $_ -gt 0 })
    if ($ft.Count -lt 10) { return $null }
    $sorted = @($ft | Sort-Object -Descending)
    $avg = ($ft | Measure-Object -Average).Average
    $worst = { param($pct) $n = [math]::Max(1, [int][math]::Floor($sorted.Count * $pct)); 1000 / (($sorted | Select-Object -First $n | Measure-Object -Average).Average) }
    $median = @($ft | Sort-Object)[[int]($ft.Count / 2)]
    return [ordered]@{
        Frames   = $ft.Count
        AvgFps   = [math]::Round(1000 / $avg, 1)
        Low1     = [math]::Round((& $worst 0.01), 1)
        Low01    = [math]::Round((& $worst 0.001), 1)
        P99Ms    = [math]::Round($sorted[[int][math]::Floor($sorted.Count * 0.01)], 2)
        Stutters = @($ft | Where-Object { $_ -gt 2 * $median }).Count
    }
}

function Measure-FrameTimes {
    # Records the named process for $Seconds with PresentMon. The game must be running and drawing frames.
    param([string]$Process, [int]$Seconds = 30)
    $pm = Get-PresentMonPath
    $csv = Join-Path $global:OptiToolsDir ('frames-{0}.csv' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
    $name = if ($Process -like '*.exe') { $Process } else { "$Process.exe" }
    $o = Invoke-Native $pm @('--process_name', $name, '--output_file', $csv, '--timed', "$Seconds", '--terminate_after_timed', '--stop_existing_session', '--no_console_stats', '--session_name', 'Optimaxer', '--v2_metrics')
    if (-not (Test-Path -LiteralPath $csv)) { throw "no frames from $name in $Seconds s. Keep the game running in the foreground while it records. $($o.Trim())" }
    $rows = Import-Csv -LiteralPath $csv
    $col = if ($rows.Count -and $rows[0].PSObject.Properties.Name -contains 'FrameTime') { 'FrameTime' } else { 'MsBetweenPresents' }
    $ft = foreach ($r in $rows) { $v = 0.0; if ([double]::TryParse([string]$r.$col, [Globalization.NumberStyles]::Float, [Globalization.CultureInfo]::InvariantCulture, [ref]$v)) { $v } }
    Remove-Item -LiteralPath $csv -Force -ErrorAction SilentlyContinue
    $st = Get-FrameStats @($ft)
    if (-not $st) { throw "only $(@($ft).Count) frames from $name; is it in the foreground?" }
    $st.Process = $name; $st.Seconds = $Seconds
    return $st
}

function Initialize-OptiTimer {
    if ('OptiTimer' -as [type]) { return }
    Add-Type -TypeDefinition @'
using System.Runtime.InteropServices;
public static class OptiTimer {
    [DllImport("ntdll.dll")] static extern int NtQueryTimerResolution(out uint min, out uint max, out uint cur);
    // current system timer resolution in milliseconds (100 ns units from ntdll)
    public static double CurrentMs() { uint a, b, c; return NtQueryTimerResolution(out a, out b, out c) == 0 ? c / 10000.0 : -1; }
}
'@
}

function Measure-Latency {
    # Timer resolution, DPC/interrupt CPU time over $Seconds, and ping/jitter to 1.1.1.1.
    param([int]$Seconds = 10, [string]$PingHost = '1.1.1.1')
    Initialize-OptiTimer
    $dpc = @(); $isr = @()
    for ($i = 0; $i -lt $Seconds; $i++) {
        try {
            $p = Get-CimInstance Win32_PerfFormattedData_Counters_ProcessorInformation -Filter "Name='_Total'" -ErrorAction Stop
            $dpc += [double]$p.PercentDPCTime; $isr += [double]$p.PercentInterruptTime
        } catch {}
        Start-Sleep -Seconds 1
    }
    $ms = @()
    try { $ms = @(Test-Connection -ComputerName $PingHost -Count 20 -ErrorAction Stop | ForEach-Object { [double]$_.ResponseTime }) } catch {}
    $jit = if ($ms.Count -gt 1) { ((1..($ms.Count - 1) | ForEach-Object { [math]::Abs($ms[$_] - $ms[$_ - 1]) }) | Measure-Object -Average).Average } else { $null }
    $avg = { param($a) if (@($a).Count) { [math]::Round((@($a) | Measure-Object -Average).Average, 2) } else { $null } }
    return [ordered]@{
        TimerMs  = [math]::Round([OptiTimer]::CurrentMs(), 3)
        DpcPct   = & $avg $dpc
        DpcMax   = $(if ($dpc.Count) { [math]::Round(($dpc | Measure-Object -Maximum).Maximum, 2) } else { $null })
        IsrPct   = & $avg $isr
        PingMs   = & $avg $ms
        JitterMs = $(if ($null -ne $jit) { [math]::Round($jit, 2) } else { $null })
        PingLost = 20 - $ms.Count
    }
}

function Get-LastBootMs {
    try {
        $e = Get-WinEvent -FilterHashtable @{ LogName = 'Microsoft-Windows-Diagnostics-Performance/Operational'; Id = 100 } -MaxEvents 1 -ErrorAction Stop
        return @{ Ms = [int](Get-EventData $e)['BootTime']; At = $e.TimeCreated.ToString('s') }
    } catch { return @{ Ms = $null; At = $null } }
}

function Get-OptiMeasurements {
    if (-not (Test-Path -LiteralPath $global:OptiMeasurePath)) { return @() }
    # PowerShell 5.1 hands a JSON array over as one object; the ForEach unrolls it
    try { return @(Get-Content -LiteralPath $global:OptiMeasurePath -Raw | ConvertFrom-Json | ForEach-Object { $_ }) } catch { return @() }
}

function Save-OptiMeasurement {
    # One labelled run ('before' or 'after'): latency and boot always, frames when a process is given.
    param([ValidateSet('before', 'after')][string]$Label, [string]$Process, [int]$Seconds = 30)
    $run = [ordered]@{ Label = $Label; Time = (Get-Date).ToString('s'); Boot = (Get-LastBootMs); Latency = (Measure-Latency); Frames = $null; FramesError = '' }
    if ($Process) { try { $run.Frames = Measure-FrameTimes $Process $Seconds } catch { $run.FramesError = $_.Exception.Message; Write-Log "  frames: $($run.FramesError)" 'WARN' } }
    $all = @(Get-OptiMeasurements) + @([pscustomobject]$run)
    ConvertTo-Json -InputObject @($all | Select-Object -Last 40) -Depth 6 | Set-Content -LiteralPath $global:OptiMeasurePath -Encoding UTF8
    $l = $run.Latency
    Write-Log ("Measured ({0}): timer {1} ms, DPC {2}% (max {3}%), interrupts {4}%, ping {5} ms, jitter {6} ms{7}" -f $Label, $l.TimerMs, $l.DpcPct, $l.DpcMax, $l.IsrPct, $l.PingMs, $l.JitterMs, $(if ($run.Boot.Ms) { (', last boot {0:N1} s' -f ($run.Boot.Ms / 1000)) } else { '' })) 'OK'
    if ($run.Frames) { $f = $run.Frames; Write-Log ("  {0}: {1} FPS average, 1% low {2}, 0.1% low {3}, 99th pct frame {4} ms, {5} stutters ({6} frames)" -f $f.Process, $f.AvgFps, $f.Low1, $f.Low01, $f.P99Ms, $f.Stutters, $f.Frames) 'OK' }
    return $run
}

function Compare-OptiMeasurements {
    # Latest 'before' against latest 'after' (same game for frames). Lines like "Average FPS: 141.2 -> 150.8 (+6.8%)".
    $all = @(Get-OptiMeasurements)
    $b = @($all | Where-Object Label -eq 'before')[-1]; $a = @($all | Where-Object Label -eq 'after')[-1]
    if (-not $b -or -not $a) { return @() }
    $line = { param($name, $x, $y, $unit, [bool]$higherBetter)
        if ($null -eq $x -or $null -eq $y) { return }
        $d = if ([double]$x) { 100 * ([double]$y - [double]$x) / [double]$x } else { 0 }
        $better = if ($higherBetter) { $y -gt $x } else { $y -lt $x }
        '{0}: {1}{4} -> {2}{4} ({3:+0.0;-0.0;0}%{5})' -f $name, $x, $y, $d, $unit, $(if ([math]::Abs($d) -lt 2) { ', same' } elseif ($better) { ', better' } else { ', worse' })
    }
    $out = @()
    if ($b.Frames -and $a.Frames -and $b.Frames.Process -eq $a.Frames.Process) {
        $out += & $line "Average FPS ($($a.Frames.Process))" $b.Frames.AvgFps $a.Frames.AvgFps '' $true
        $out += & $line '1% low FPS' $b.Frames.Low1 $a.Frames.Low1 '' $true
        $out += & $line '0.1% low FPS' $b.Frames.Low01 $a.Frames.Low01 '' $true
        $out += & $line '99th percentile frame time' $b.Frames.P99Ms $a.Frames.P99Ms ' ms' $false
    }
    $out += & $line 'DPC time' $b.Latency.DpcPct $a.Latency.DpcPct '%' $false
    $out += & $line 'Interrupt time' $b.Latency.IsrPct $a.Latency.IsrPct '%' $false
    $out += & $line 'Ping jitter' $b.Latency.JitterMs $a.Latency.JitterMs ' ms' $false
    if ($b.Boot.Ms -and $a.Boot.Ms -and $b.Boot.At -ne $a.Boot.At) { $out += & $line 'Boot time' ([math]::Round($b.Boot.Ms / 1000, 1)) ([math]::Round($a.Boot.Ms / 1000, 1)) ' s' $false }
    return @($out | Where-Object { $_ })
}

function Get-EvidenceText {
    $cmp = @(Compare-OptiMeasurements)
    if ($cmp.Count) { return 'Measured on this PC, before vs after: ' + ($cmp -join '; ') + '.' }
    $all = @(Get-OptiMeasurements)
    if ($all.Count) { return "Measured 'before' on $([datetime]$all[-1].Time). Optimize, restart, play the same scene, then measure 'after'." }
    return 'No measurements yet. Tools > Measure records frame times of a game (PresentMon), DPC/interrupt load, timer, ping jitter and boot time.'
}

function Undo-OptiFeatures {
    # Used by Undo ALL. Returns $false when something could not be put back.
    $ok = $true
    if (-not (Remove-GameProfile)) { $ok = $false }
    try { if (-not (Remove-SearchExclusions)) { $ok = $false } } catch { $ok = $false }
    if (-not (Restore-WakeSources)) { $ok = $false }
    return $ok
}
