# Optimaxer UI: window, pages, background jobs. Dot-sourced by Optimaxer.ps1 (needs $global:OptiRoot).

Add-Type -AssemblyName PresentationFramework, PresentationCore, WindowsBase, System.Xaml, Microsoft.VisualBasic
. "$global:OptiRoot\src\Types.ps1"

$global:OptiLogQueue = New-Object 'System.Collections.Concurrent.ConcurrentQueue[string]'
$global:Jobs    = New-Object System.Collections.ArrayList
$global:Busy    = $false
$global:Loaded  = @{}
$global:Sys     = @{ Laptop = $false; SSD = $true }
$global:OptiFilters = @{}

# ================================================================ window
$xamlText = Get-Content -LiteralPath "$global:OptiRoot\src\MainWindow.xaml" -Raw -Encoding UTF8
$win = [Windows.Markup.XamlReader]::Parse($xamlText)
$UI = @{}
foreach ($m in [regex]::Matches($xamlText, 'x:Name="(\w+)"')) { $UI[$m.Groups[1].Value] = $win.FindName($m.Groups[1].Value) }
$global:UI = $UI
$global:Win = $win
. "$global:OptiRoot\src\Theme.ps1"

# ================================================================ helpers
function New-Row {
    param($Id, $Name, $Desc, $Group = '', $Risk = '', $Extra = '', [bool]$Checked = $false, $Tag = $null)
    $r = New-Object OptiRow
    $r.Id = $Id; $r.Name = $Name; $r.Desc = $Desc; $r.Group = [string]$Group; $r.Risk = $Risk; $r.Extra = $Extra; $r.IsChecked = $Checked; $r.Tag = $Tag
    return $r
}

function New-RowFilter {
    param([string]$Key)
    return [Predicate[object]]([scriptblock]::Create(
        "param(`$o) `$t = `$global:OptiFilters['$Key']; if ([string]::IsNullOrEmpty(`$t)) { return `$true }; " +
        "([string]`$o.Name).IndexOf(`$t, [StringComparison]::OrdinalIgnoreCase) -ge 0 -or ([string]`$o.Desc).IndexOf(`$t, [StringComparison]::OrdinalIgnoreCase) -ge 0"))
}

function Initialize-List {
    param($ListBox, [string]$Key)
    $col = New-Object 'System.Collections.ObjectModel.ObservableCollection[OptiRow]'
    $ListBox.ItemsSource = $col
    $view = [System.Windows.Data.CollectionViewSource]::GetDefaultView($col)
    $view.GroupDescriptions.Add((New-Object System.Windows.Data.PropertyGroupDescription 'Group'))
    $ListBox.GroupStyle.Add([Windows.Markup.XamlReader]::Parse(
        '<GroupStyle xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"><GroupStyle.HeaderTemplate><DataTemplate>' +
        '<TextBlock Text="{Binding Name}" FontWeight="Bold" FontSize="13" Foreground="{DynamicResource Text}" Margin="4,14,0,6"/></DataTemplate></GroupStyle.HeaderTemplate></GroupStyle>'))
    $global:OptiFilters[$Key] = ''
    $view.Filter = New-RowFilter $Key
    return , $col
}

function Connect-Search {
    param($TextBox, [string]$Key, $Collection)
    $TextBox.Add_TextChanged({
        param($s, $e)
        $global:OptiFilters[$s.Tag.Key] = $s.Text
        [System.Windows.Data.CollectionViewSource]::GetDefaultView($s.Tag.Col).Refresh()
    })
    $TextBox.Tag = @{ Key = $Key; Col = $Collection }
}

function Test-Elevated {
    if (([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { return $true }
    [void][System.Windows.MessageBox]::Show('This action changes system settings and needs administrator rights. Close Optimaxer and start it with Launch.bat (accept the UAC prompt).', 'Optimaxer', 'OK', 'Warning')
    return $false
}

function Confirm-Action {
    param([string]$Text, [string]$Title = 'Optimaxer')
    return ([System.Windows.MessageBox]::Show($Text, $Title, 'YesNo', 'Question') -eq 'Yes')
}

function Set-Busy {
    param([bool]$On, [string]$Text = '')
    $global:Busy = $On
    $UI.PagesHost.IsEnabled = -not $On
    $UI.PagesHost.Opacity = if ($On) { 0.55 } else { 1 }
    $UI.BusyBar.Visibility = if ($On) { 'Visible' } else { 'Hidden' }
    $UI.BusyBar.Value = if ($On) { 100 } else { 0 }
    $UI.BusyText.Text = if ($On) { "Working: $Text" } else { 'Log' }
    if ($On) {
        $a = New-Object Windows.Media.Animation.DoubleAnimation(0.25, 1, [TimeSpan]::FromMilliseconds(700))
        $a.AutoReverse = $true
        $a.RepeatBehavior = [Windows.Media.Animation.RepeatBehavior]::Forever
        $UI.BusyBar.BeginAnimation([Windows.UIElement]::OpacityProperty, $a)
    } else { $UI.BusyBar.BeginAnimation([Windows.UIElement]::OpacityProperty, $null) }
}

function Start-OptiJob {
    param([string]$Title, [scriptblock]$Script, [object[]]$ArgList = @(), [scriptblock]$OnDone)
    if ($global:Busy) { Write-Log 'Another task is running, please wait.' 'WARN'; return $false }
    Set-Busy $true $Title
    Write-Log "== $Title"
    $rs = [runspacefactory]::CreateRunspace()
    $rs.ApartmentState = 'MTA'
    $rs.Open()
    $rs.SessionStateProxy.SetVariable('OptiLogQueue', $global:OptiLogQueue)
    $rs.SessionStateProxy.SetVariable('OptiRoot', $global:OptiRoot)
    $ps = [powershell]::Create()
    $ps.Runspace = $rs
    $boot = {
        param($sbText, $argv)
        $ErrorActionPreference = 'Continue'
        . "$OptiRoot\src\Core.ps1"; . "$OptiRoot\src\Data.ps1"; . "$OptiRoot\src\Catalog.ps1"
        & ([scriptblock]::Create($sbText)) @argv
    }
    [void]$ps.AddScript($boot.ToString()).AddArgument($Script.ToString()).AddArgument($ArgList)
    [void]$global:Jobs.Add(@{ PS = $ps; RS = $rs; Handle = $ps.BeginInvoke(); OnDone = $OnDone; Title = $Title })
    return $true
}

function Complete-OptiJobs {
    foreach ($j in @($global:Jobs)) {
        if (-not $j.Handle.IsCompleted) { continue }
        $res = @()
        try { $res = @($j.PS.EndInvoke($j.Handle)) } catch { Write-Log "$($j.Title) failed: $($_.Exception.Message)" 'ERROR' }
        foreach ($e in $j.PS.Streams.Error) {
            $where = if ($e.InvocationInfo -and $e.InvocationInfo.Line) { ' [' + $e.InvocationInfo.Line.Trim() + ']' } else { '' }
            $msg = "  ! $($e.ToString())$where"
            Write-Log ($msg.Substring(0, [math]::Min(300, $msg.Length))) 'WARN'
        }
        $j.PS.Dispose(); $j.RS.Close(); $j.RS.Dispose()
        $global:Jobs.Remove($j)
        Set-Busy $false
        if ($j.OnDone) { try { & $j.OnDone $res } catch { Write-Log "UI update failed: $($_.Exception.Message)" 'ERROR' } }
    }
}

function Update-LogPane {
    $sb = New-Object System.Text.StringBuilder
    $line = $null
    while ($global:OptiLogQueue.TryDequeue([ref]$line)) { [void]$sb.AppendLine($line) }
    if ($sb.Length) { $UI.Log.AppendText($sb.ToString()); $UI.Log.ScrollToEnd() }
}

function Show-Page {
    param([string]$Name)
    foreach ($k in @($UI.Keys)) { if ($k -like 'Page_*') { $UI[$k].Visibility = if ($k -eq "Page_$Name") { 'Visible' } else { 'Collapsed' } } }
    $titles = @{ dash = 'dashboard'; tweaks = 'optimize'; services = 'services'; startup = 'startup'; cleanup = 'cleaner'; debloat = 'debloat'; network = 'network'; features = 'features'; tools = 'tools'; rice = 'rice' }
    $UI.BarTitle.Text = "~/$($titles[$Name])"
    Start-PageIn $UI["Page_$Name"]
    if (-not $global:Loaded[$Name] -and -not $global:Busy) {
        switch ($Name) {
            'services' { Update-Services }
            'startup'  { Update-Startup }
            'features' { Update-Features }
            'network'  { Show-CurrentDns }
        }
    }
}

# ================================================================ dashboard
$cpuCounter = $null; $upCounter = $null; $compInfo = $null
try {
    $cpuCounter = New-Object Diagnostics.PerformanceCounter('Processor', '% Processor Time', '_Total'); [void]$cpuCounter.NextValue()
    $upCounter  = New-Object Diagnostics.PerformanceCounter('System', 'System Up Time'); [void]$upCounter.NextValue()
    $compInfo   = New-Object Microsoft.VisualBasic.Devices.ComputerInfo
} catch {}
$sysDrive = New-Object IO.DriveInfo $env:SystemDrive
$UI.DiskLabel.Text = "Disk ($($env:SystemDrive))"

function Update-Live {
    try {
        $UI.BarClock.Text = (Get-Date).ToString('HH:mm')
        $c = if ($cpuCounter) { [math]::Round($cpuCounter.NextValue()) } else { 0 }
        $UI.BarCpu.Text = 'CPU {0}%' -f $c
        if ($compInfo) {
            $tot = [double]$compInfo.TotalPhysicalMemory; $av = [double]$compInfo.AvailablePhysicalMemory
            $UI.BarRam.Text = 'RAM {0:N1} GB' -f (($tot - $av) / 1GB)
        }
        if ($UI.Page_dash.Visibility -ne 'Visible') { return }
        $UI.CpuText.Text = "$c%"; Set-Bar $UI.CpuBar $c
        if ($compInfo) {
            $p = [math]::Round(100 * ($tot - $av) / $tot)
            $UI.RamText.Text = '{0:N1} / {1:N0} GB' -f (($tot - $av) / 1GB), ($tot / 1GB); Set-Bar $UI.RamBar $p
        }
        $UI.DiskText.Text = '{0:N0} GB free' -f ($sysDrive.AvailableFreeSpace / 1GB)
        Set-Bar $UI.DiskBar ([math]::Round(100 * (1 - $sysDrive.AvailableFreeSpace / $sysDrive.TotalSize)))
        if ($upCounter) {
            $ts = [TimeSpan]::FromSeconds($upCounter.NextValue())
            $UI.UpText.Text = '{0}d {1}h {2}m' -f $ts.Days, $ts.Hours, $ts.Minutes
            $UI.UpHint.Text = if ($ts.TotalDays -gt 7) { 'Restart recommended' } else { ' ' }
        }
    } catch {}
}

function Set-Score {
    param([int]$Pct, [int]$On, [int]$Total)
    $col = if ($Pct -ge 80) { 'Good' } elseif ($Pct -ge 45) { 'Accent' } else { 'Bad' }
    $UI.ScoreText.Text = "$Pct%"
    Set-Res $UI.ScoreText ([Windows.Controls.TextBlock]::ForegroundProperty) $col
    Set-Bar $UI.ScoreBar $Pct
    Set-Res $UI.ScoreBar ([Windows.Controls.Primitives.RangeBase]::ForegroundProperty) $(if ($Pct -ge 80) { 'Good' } elseif ($Pct -ge 45) { 'AccentGrad' } else { 'Bad' })
    $UI.ScoreHead.Text = if ($Pct -ge 80) { 'Well tuned' } elseif ($Pct -ge 45) { 'Partially optimized' } else { 'Not optimized yet' }
    $UI.ScoreSub.Text = "$On of $Total recommended tweaks are active. Every change is recorded and can be undone."
}

function Update-Score {
    $applicable = @($global:TwRows | Where-Object { $_.Tag.Tags -contains 'safe' -and (Test-Applicable $_.Tag) })
    if (-not $applicable.Count) { return }
    $on = @($applicable | Where-Object { $_.Extra -eq 'Applied' }).Count
    Set-Score ([int][math]::Round(100 * $on / $applicable.Count)) $on $applicable.Count
}

function Test-Applicable {
    param($T)
    if ($T.DesktopOnly -and $global:Sys.Laptop) { return $false }
    if ($T.SSDOnly -and -not $global:Sys.SSD) { return $false }
    return $true
}

function Initialize-Dashboard {
    Start-OptiJob 'Reading system information' -Script {
        $os = Get-CimInstance Win32_OperatingSystem; $cs = Get-CimInstance Win32_ComputerSystem
        $cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
        $ssd = $true
        try {
            $dn = (Get-Partition -DriveLetter $env:SystemDrive[0] -ErrorAction Stop).DiskNumber
            if ((Get-PhysicalDisk | Where-Object { $_.DeviceId -eq "$dn" }).MediaType -eq 'HDD') { $ssd = $false }
        } catch {}
        @{
            OS = "$($os.Caption) (build $($os.BuildNumber))"
            Machine = "$($cs.Manufacturer) $($cs.Model)"
            CPU = "$($cpu.Name.Trim()) ($($cpu.NumberOfCores) cores / $($cpu.NumberOfLogicalProcessors) threads)"
            GPU = ((Get-CimInstance Win32_VideoController | ForEach-Object Name) -join ', ')
            RAM = [math]::Round($cs.TotalPhysicalMemory / 1GB, 1)
            Laptop = [bool](Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue)
            SSD = $ssd
            Disks = @(Get-PhysicalDisk | ForEach-Object { '{0} - {1}, {2} GB, {3}' -f $_.FriendlyName, $_.MediaType, [math]::Round($_.Size / 1GB), $_.HealthStatus })
            Startup = @(Get-StartupItems).Count
        }
    } -OnDone {
        param($res)
        $i = $res | Select-Object -First 1
        if ($i) {
            $global:Sys.Laptop = [bool]$i.Laptop; $global:Sys.SSD = [bool]$i.SSD
            $pairs = @(
                , @('Host', "$($i.Machine) ($(if ($i.Laptop) { 'laptop' } else { 'desktop' }))")
                , @('OS', $i.OS)
                , @('CPU', $i.CPU)
                , @('GPU', $i.GPU)
                , @('Memory', "$($i.RAM) GB")
            ) + @($i.Disks | ForEach-Object { , @('Disk', $_) }) + @(, @('Startup', "$($i.Startup) items"))
            $UI.SysInfo.Inlines.Clear()
            foreach ($pr in $pairs) {
                $k = New-Object Windows.Documents.Run(($pr[0] + ': '))
                $k.FontWeight = 'SemiBold'
                $k.SetResourceReference([Windows.Documents.TextElement]::ForegroundProperty, 'Accent2')
                [void]$UI.SysInfo.Inlines.Add($k)
                [void]$UI.SysInfo.Inlines.Add((New-Object Windows.Documents.Run(([string]$pr[1] + "`n"))))
            }
        }
        Update-TweakStatus
    }
}

# ================================================================ tweaks page
$global:TwRows = New-Object 'System.Collections.ObjectModel.ObservableCollection[OptiRow]'
$global:TwViews = New-Object System.Collections.ArrayList
$global:OptiFilters['tw'] = ''
$UI.TwSearch.Add_TextChanged({
    $global:OptiFilters['tw'] = $UI.TwSearch.Text
    foreach ($v in $global:TwViews) { $v.Refresh() }
})
foreach ($t in $global:OptiCatalog) {
    $notes = @()
    if ($t.DesktopOnly) { $notes += 'desktop PCs' }
    if ($t.SSDOnly) { $notes += 'SSD only' }
    if ($t.Restart) { $notes += 'restart needed' }
    if ($t.Tags -contains 'optin') { $notes += 'opt-in, never in presets' }
    $d = if ($notes) { "$($t.Desc)  [$($notes -join ', ')]" } else { $t.Desc }
    $global:TwRows.Add((New-Row $t.Id $t.Name $d $t.Cat $t.Risk '' $false $t))
}
$colSize = @(0, 0, 0)
$tweakCols = @($UI.TwCol1, $UI.TwCol2, $UI.TwCol3)
foreach ($cat in @($global:TwRows | Group-Object Group)) {
    $i = 0; for ($j = 1; $j -lt 3; $j++) { if ($colSize[$j] -lt $colSize[$i]) { $i = $j } }
    $colSize[$i] += $cat.Count + 2
    $head = New-Object Windows.Controls.TextBlock -Property @{ Text = $cat.Name }
    $head.Style = $win.FindResource('ColHead')
    [void]$tweakCols[$i].Children.Add($head)
    $col = New-Object 'System.Collections.ObjectModel.ObservableCollection[OptiRow]'
    foreach ($r in $cat.Group) { $col.Add($r) }
    $ic = New-Object Windows.Controls.ItemsControl
    $ic.ItemTemplate = $win.FindResource('TweakTemplate')
    $ic.ItemsSource = $col
    $view = [System.Windows.Data.CollectionViewSource]::GetDefaultView($col)
    $view.Filter = New-RowFilter 'tw'
    [void]$global:TwViews.Add($view)
    [void]$tweakCols[$i].Children.Add($ic)
}

function Update-TweakStatus {
    [void](Start-OptiJob 'Checking active tweaks' -Script { Get-TweakStatus } -OnDone {
        param($res)
        $map = @{}; foreach ($r in $res) { $map[$r.Id] = [bool]$r.On }
        foreach ($row in $global:TwRows) { $row.Extra = if ($map[$row.Id]) { 'Applied' } else { '' } }
        $global:Loaded['tweaks'] = $true
        Update-Score
    })
}

function Set-Preset {
    param([string]$Tag)
    foreach ($row in $global:TwRows) {
        $t = $row.Tag
        $row.IsChecked = ($Tag -ne 'none') -and ($t.Tags -contains $Tag) -and (Test-Applicable $t)
    }
}

function Invoke-TweakJob {
    param([ValidateSet('apply', 'undo')][string]$Mode)
    $sel = @($global:TwRows | Where-Object IsChecked)
    if (-not $sel.Count) { Write-Log 'No tweaks selected.' 'WARN'; return }
    if (-not (Test-Elevated)) { return }
    if ($Mode -eq 'apply') {
        $adv = @($sel | Where-Object { $_.Risk -eq 'Advanced' } | ForEach-Object Name)
        $msg = "Apply $($sel.Count) tweak(s)?"
        if ($adv) { $msg += "`n`nAdvanced tweaks selected (read their descriptions):`n - " + ($adv -join "`n - ") }
        if (-not (Confirm-Action $msg)) { return }
    }
    [void](Start-OptiJob "$Mode $($sel.Count) tweak(s)" -ArgList @(@($sel | ForEach-Object Id), $Mode, [bool]$UI.ChkRestore.IsChecked) -Script {
        param($ids, $mode, $restorePoint)
        if ($mode -eq 'apply' -and $restorePoint) { [void](New-OptiRestorePoint 'Optimaxer - before tweaks') }
        $restart = $false; $explorer = $false; $ok = 0
        foreach ($id in $ids) {
            $t = Get-TweakById $id
            if (-not $t) { continue }
            $on = if ($mode -eq 'apply') { Invoke-TweakApply $t } else { Invoke-TweakUndo $t }
            if ($t.Restart) { $restart = $true }
            if ($t.Explorer) { $explorer = $true }
            if (($mode -eq 'apply') -eq [bool]$on) { $ok++ }
            @{ Id = $id; On = [bool]$on }
        }
        if ($explorer) { Write-Log 'Restarting Explorer to apply interface changes'; Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue }
        Write-Log "Finished: $ok of $($ids.Count) tweak(s) verified." 'OK'
        if ($restart) { Write-Log 'Restart Windows for all changes to take full effect.' 'OK' }
    } -OnDone {
        param($res)
        $map = @{}; foreach ($r in $res) { $map[$r.Id] = [bool]$r.On }
        foreach ($row in $global:TwRows) { if ($map.ContainsKey($row.Id)) { $row.Extra = if ($map[$row.Id]) { 'Applied' } else { '' } } }
        Update-Score
    })
}

# ================================================================ services page
$global:SvRows = Initialize-List $UI.SvList 'sv'
Connect-Search $UI.SvSearch 'sv' $global:SvRows

function Update-Services {
    [void](Start-OptiJob 'Reading services' -Script {
        $recs = @{}; foreach ($r in $OptiServiceRecs) { $recs[$r.Name] = $r }
        foreach ($s in Get-CimInstance Win32_Service) {
            if ($s.Name -match '_[0-9a-f]{4,}$' -or $OptiProtectedServices -contains $s.Name) { continue }
            $mode = $s.StartMode
            if ($mode -eq 'Auto' -and $s.DelayedAutoStart) { $mode = 'Auto (delayed)' }
            $r = $recs[$s.Name]
            @{ Name = $s.Name; Display = $s.DisplayName; Desc = [string]$s.Description; Mode = $mode; State = $s.State
               Rec = $(if ($r) { [int]$r.Start } else { 0 }); RecDesc = $(if ($r) { $r.Desc } else { '' }) }
        }
    } -OnDone {
        param($res)
        $items = @($res | Sort-Object @{ e = { if ($_.Rec -eq 3) { 0 } elseif ($_.Rec -eq 4) { 1 } else { 2 } } }, @{ e = { $_.Display } })
        $global:SvRows.Clear()
        foreach ($s in $items) {
            $grp = switch ($s.Rec) { 3 { 'Recommended: set to Manual' } 4 { 'Recommended: disable' } default { 'Other services' } }
            $risk = switch ($s.Rec) { 3 { 'Safe' } 4 { 'Moderate' } default { '' } }
            $desc = if ($s.RecDesc) { $s.RecDesc } else { $s.Desc }
            if ($desc.Length -gt 170) { $desc = $desc.Substring(0, 167) + '...' }
            $global:SvRows.Add((New-Row $s.Name $(if ($s.Display -eq $s.Name) { $s.Name } else { "$($s.Display)  ($($s.Name))" }) $desc $grp $risk "$($s.Mode) / $($s.State)" $false ([int]$s.Rec)))
        }
        $global:Loaded['services'] = $true
        Write-Log "Loaded $($global:SvRows.Count) services."
    })
}

function Set-ServiceStartJob {
    param([int]$Start, [string]$Label)
    $names = @($global:SvRows | Where-Object IsChecked | ForEach-Object Id)
    if (-not $names.Count) { Write-Log 'No services selected.' 'WARN'; return }
    if (-not (Test-Elevated)) { return }
    if (-not (Confirm-Action "Set $($names.Count) service(s) to ${Label}?`nA backup is saved first so you can restore.")) { return }
    [void](Start-OptiJob "Services to $Label" -ArgList @($names, $Start) -Script {
        param($names, $start)
        $backup = @(); foreach ($n in $names) { $s = Get-SvcSnapshot $n; if ($s) { $backup += $s } }
        $file = Join-Path $OptiBackupDir ('services-{0}.json' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
        $backup | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $file -Encoding UTF8
        Write-Log "Backup saved: $file"
        foreach ($b in $backup) {
            if (Set-SvcStart $b.Name $start) { Write-Log "  $($b.Name) updated" } else { Write-Log "  $($b.Name): refused (protected by Windows)" 'WARN' }
        }
    } -OnDone { Update-Services })
}

# ================================================================ startup page
$global:StRows = Initialize-List $UI.StList 'st'

function Update-Startup {
    [void](Start-OptiJob 'Reading startup items' -Script { Get-StartupItems } -OnDone {
        param($res)
        $global:StRows.Clear()
        foreach ($i in $res) {
            $item = @{ Name = [string]$i.Name; Approved = [string]$i.Approved }
            $global:StRows.Add((New-Row "$($i.Scope)|$($i.Name)" $i.Name $i.Cmd $i.Scope '' $(if ($i.Enabled) { 'Enabled' } else { 'Disabled' }) $false $item))
        }
        $global:Loaded['startup'] = $true
        Write-Log "Found $($global:StRows.Count) startup items."
    })
}

function Set-StartupJob {
    param([bool]$Enable)
    $sel = @($global:StRows | Where-Object IsChecked)
    if (-not $sel.Count) { Write-Log 'No startup items selected.' 'WARN'; return }
    $items = @($sel | ForEach-Object { @{ Name = $_.Tag.Name; Approved = $_.Tag.Approved } })
    [void](Start-OptiJob "$(if ($Enable) { 'Enable' } else { 'Disable' }) startup items" -ArgList @($items, $Enable) -Script {
        param($items, $enable)
        foreach ($i in $items) { Set-StartupItem $i $enable; Write-Log "  $($i.Name): $(if ($enable) { 'enabled' } else { 'disabled' })" }
    } -OnDone { Update-Startup })
}

# ================================================================ cleaner page
$global:ClRows = Initialize-List $UI.ClList 'cl'
foreach ($c in $global:OptiCleanup) {
    $risk = if ($c.Risk) { $c.Risk } else { 'Safe' }
    $global:ClRows.Add((New-Row $c.Id $c.Name $c.Desc 'Cleanup targets' $risk '' $false $null))
}

function Start-CleanScan {
    [void](Start-OptiJob 'Scanning for junk' -Script {
        foreach ($c in $OptiCleanup) { @{ Id = $c.Id; Size = (Get-CleanupSize $c) } }
    } -OnDone {
        param($res)
        $map = @{}; foreach ($r in $res) { $map[$r.Id] = [double]$r.Size }
        $total = 0.0
        foreach ($row in $global:ClRows) {
            $s = $map[$row.Id]
            $row.Extra = if ($s -lt 0) { 'on demand' } elseif ($s -le 0) { 'empty' } else { Format-Size $s }
            $row.IsChecked = ($s -gt 0) -and ($row.Risk -eq 'Safe')
            if ($s -gt 0) { $total += $s }
        }
        $UI.ClTotal.Text = "Reclaimable: $(Format-Size $total)"
        Write-Log "Scan finished: $(Format-Size $total) can be reclaimed."
    })
}

function Start-Clean {
    $ids = @($global:ClRows | Where-Object IsChecked | ForEach-Object Id)
    if (-not $ids.Count) { Write-Log 'Nothing selected. Scan first.' 'WARN'; return }
    if (-not (Test-Elevated)) { return }
    if (-not (Confirm-Action "Delete the selected items? This cannot be undone.")) { return }
    [void](Start-OptiJob 'Cleaning' -ArgList @(, $ids) -Script {
        param($ids)
        $sum = 0.0
        foreach ($id in $ids) {
            $c = $OptiCleanup | Where-Object { $_.Id -eq $id }
            $f = Invoke-CleanupTarget $c
            $sum += $f
            Write-Log ("  {0}: freed {1}" -f $c.Name, (Format-Size $f))
        }
        Write-Log "Total freed: $(Format-Size $sum)" 'OK'
    } -OnDone { Start-CleanScan })
}

# ================================================================ debloat page
$global:DbRows = Initialize-List $UI.DbList 'db'

function Start-AppScan {
    [void](Start-OptiJob 'Scanning installed apps' -Script {
        Get-AppxPackage -AllUsers | Group-Object Name | ForEach-Object {
            $p = $_.Group[0]
            if ($p.NonRemovable -or $p.IsFramework) { return }
            $m = $OptiAppx | Where-Object { $p.Name -like $_.Id } | Select-Object -First 1
            if ($m) { @{ Name = $p.Name; Friendly = $m.Name; Level = $m.Level } }
        }
    } -OnDone {
        param($res)
        $global:DbRows.Clear()
        foreach ($a in @($res | Sort-Object @{ e = { $_.Level } }, @{ e = { $_.Friendly } })) {
            $grp = switch ($a.Level) { 'R' { 'Recommended to remove' } 'O' { 'Optional: you may use these' } default { 'Caution' } }
            $risk = switch ($a.Level) { 'R' { 'Safe' } 'O' { 'Moderate' } default { 'Advanced' } }
            $global:DbRows.Add((New-Row $a.Name $a.Friendly $a.Name $grp $risk '' ($a.Level -eq 'R') $null))
        }
        Write-Log "Found $($global:DbRows.Count) removable apps."
    })
}

function Start-AppRemove {
    $names = @($global:DbRows | Where-Object IsChecked | ForEach-Object Id)
    if (-not $names.Count) { Write-Log 'No apps selected.' 'WARN'; return }
    if (-not (Test-Elevated)) { return }
    if (-not (Confirm-Action "Remove $($names.Count) app(s) for all users?`nThey can be reinstalled from the Microsoft Store.")) { return }
    [void](Start-OptiJob 'Removing apps' -ArgList @(, $names) -Script {
        param($names)
        foreach ($n in $names) {
            try {
                Get-AppxPackage -AllUsers -Name $n | Remove-AppxPackage -AllUsers -ErrorAction Stop
                Get-AppxProvisionedPackage -Online | Where-Object DisplayName -eq $n | Remove-AppxProvisionedPackage -Online -ErrorAction SilentlyContinue | Out-Null
                Write-Log "  removed $n"
            } catch { Write-Log "  $n : $($_.Exception.Message)" 'WARN' }
        }
    } -OnDone { Start-AppScan })
}

# ================================================================ network page
function Show-CurrentDns {
    $global:Loaded['network'] = $true
    [void](Start-OptiJob 'Reading DNS settings' -Script {
        Get-DnsClientServerAddress -AddressFamily IPv4 | Where-Object { $_.ServerAddresses } | ForEach-Object { Write-Log ("  {0}: {1}" -f $_.InterfaceAlias, ($_.ServerAddresses -join ', ')) }
    })
}

foreach ($p in $global:OptiDns) {
    $b = New-Object Windows.Controls.Button
    $b.Width = 236; $b.Margin = '0,0,8,8'; $b.Padding = '12,9'
    $b.HorizontalContentAlignment = 'Left'
    $sp = New-Object Windows.Controls.StackPanel
    $t1 = New-Object Windows.Controls.TextBlock -Property @{ Text = $p.Name; FontWeight = 'SemiBold'; FontSize = 13 }
    $t2 = New-Object Windows.Controls.TextBlock -Property @{ Text = $p.Desc; TextWrapping = 'Wrap'; FontSize = 11; Margin = '0,3,0,0' }
    Set-Res $t2 ([Windows.Controls.TextBlock]::ForegroundProperty) 'Muted'
    [void]$sp.Children.Add($t1); [void]$sp.Children.Add($t2)
    $b.Content = $sp
    $b.Tag = $p
    $b.Add_Click({
        param($s, $e)
        $d = $s.Tag
        if (-not (Test-Elevated)) { return }
        if (-not (Confirm-Action "Set DNS to $($d.Name) on all active network adapters?")) { return }
        [void](Start-OptiJob "DNS: $($d.Name)" -ArgList @($d.Name, ([string[]]$d.V4), ([string[]]$d.V6), $d.Doh) -Script {
            param($name, $v4, $v6, $doh) Set-DnsProvider -Name $name -V4 $v4 -V6 $v6 -Doh $doh
        })
    })
    [void]$UI.DnsPanel.Children.Add($b)
}

# ================================================================ features page
$global:FtRows = Initialize-List $UI.FtList 'ft'

function Update-Features {
    [void](Start-OptiJob 'Reading Windows features' -Script {
        Get-WindowsOptionalFeature -Online | ForEach-Object { @{ Id = $_.FeatureName; State = [string]$_.State } }
    } -OnDone {
        param($res)
        $map = @{}; foreach ($r in $res) { $map[$r.Id] = $r.State }
        $global:FtRows.Clear()
        foreach ($f in $global:OptiFeatures) {
            if (-not $map.ContainsKey($f.Id)) { continue }
            $global:FtRows.Add((New-Row $f.Id $f.Name $f.Desc 'Optional features' '' $map[$f.Id] $false $null))
        }
        $global:Loaded['features'] = $true
    })
}

function Set-FeatureJob {
    param([bool]$Enable)
    $ids = @($global:FtRows | Where-Object IsChecked | ForEach-Object Id)
    if (-not $ids.Count) { Write-Log 'No features selected.' 'WARN'; return }
    if (-not (Test-Elevated)) { return }
    [void](Start-OptiJob "$(if ($Enable) { 'Enable' } else { 'Disable' }) features" -ArgList @($ids, $Enable) -Script {
        param($ids, $enable)
        foreach ($id in $ids) {
            try {
                if ($enable) { Enable-WindowsOptionalFeature -Online -FeatureName $id -All -NoRestart -ErrorAction Stop | Out-Null }
                else { Disable-WindowsOptionalFeature -Online -FeatureName $id -NoRestart -ErrorAction Stop | Out-Null }
                Write-Log "  $id : done (restart may be required)"
            } catch { Write-Log "  $id : $($_.Exception.Message)" 'WARN' }
        }
    } -OnDone { Update-Features })
}

# ================================================================ tools page
$global:Tools = [ordered]@{
    restore = @{ Title = 'Create restore point';        Desc = 'Snapshot system settings so you can roll back from Windows System Restore.' }
    ram     = @{ Title = 'Free RAM now';                Desc = 'Trims process working sets and purges the standby list. Helps when memory is nearly full.' }
    drives  = @{ Title = 'Trim SSDs';                   Desc = 'Runs TRIM on every SSD. Takes seconds.' }
    defrag  = @{ Title = 'Defragment hard disks';       Desc = 'Slow: can take an hour or more on large HDDs. Runs in the background.' }
    health  = @{ Title = 'Disk health report';          Desc = 'Health, temperature, wear and error counters of your drives.' }
    sfc     = @{ Title = 'Scan system files (SFC)';     Desc = 'Repairs corrupted Windows system files. Takes several minutes.' }
    dism    = @{ Title = 'Repair Windows image (DISM)'; Desc = 'RestoreHealth: fixes the component store. Needs internet. Run before SFC if SFC fails.' }
    wu      = @{ Title = 'Reset Windows Update';        Desc = 'Clears the update cache and re-registers update services. Fixes stuck updates.' }
    icons   = @{ Title = 'Rebuild icon cache';          Desc = 'Fixes blank or wrong icons and thumbnails. Restarts Explorer.' }
    store   = @{ Title = 'Reset Microsoft Store cache'; Desc = 'Runs wsreset to fix Store download problems.' }
    perfctr = @{ Title = 'Rebuild performance counters';  Desc = 'Runs lodctr /r and winmgmt /resyncperf. Fixes empty Task Manager graphs, missing counters and slow WMI.' }
    regbackup = @{ Title = 'Export registry backup';    Desc = 'Saves .reg exports of the policy, Explorer, memory and network keys that Optimaxer edits, to the backups folder.' }
    rstrui  = @{ Title = 'Open System Restore';         Desc = 'Roll your PC back to a restore point.' }
    revert  = @{ Title = 'Undo ALL Optimaxer tweaks';   Desc = 'Restores every setting changed by this tool to its original value.'; Danger = $true }
}

$toolJobs = @{
    restore = { [void](New-OptiRestorePoint 'Optimaxer - manual') }
    ram = {
        . "$OptiRoot\src\Native.ps1"
        $before = (Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory
        $n = [OptiMem]::TrimWorkingSets(); $s = [OptiMem]::PurgeStandbyList()
        Start-Sleep -Seconds 1
        $after = (Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory
        Write-Log ("Trimmed {0} processes, standby purge {1}. Free RAM +{2}. (Windows re-caches as you work; this is mainly useful when memory is full.)" -f $n, $(if ($s) { 'ok' } else { 'skipped (needs admin)' }), (Format-Size (($after - $before) * 1KB))) 'OK'
    }
    drives = {
        foreach ($v in Get-Volume | Where-Object { $_.DriveLetter -and $_.DriveType -eq 'Fixed' }) {
            $media = 'Unspecified'
            try { $media = (Get-Partition -DriveLetter $v.DriveLetter | Get-Disk | Get-PhysicalDisk).MediaType } catch {}
            if ($media -eq 'HDD') { Write-Log "  $($v.DriveLetter): skipped (HDD, use defragment)"; continue }
            $sw = [Diagnostics.Stopwatch]::StartNew()
            try { Optimize-Volume -DriveLetter $v.DriveLetter -ReTrim -ErrorAction Stop; Write-Log ("  {0}: TRIM done in {1:N1}s" -f $v.DriveLetter, $sw.Elapsed.TotalSeconds) } catch { Write-Log "  $($v.DriveLetter): $($_.Exception.Message)" 'WARN' }
        }
        Write-Log 'Finished.' 'OK'
    }
    defrag = {
        foreach ($v in Get-Volume | Where-Object { $_.DriveLetter -and $_.DriveType -eq 'Fixed' }) {
            $media = 'Unspecified'
            try { $media = (Get-Partition -DriveLetter $v.DriveLetter | Get-Disk | Get-PhysicalDisk).MediaType } catch {}
            if ($media -ne 'HDD') { continue }
            Write-Log "  $($v.DriveLetter): defragmenting, this can take a long time..."
            $sw = [Diagnostics.Stopwatch]::StartNew()
            try { Optimize-Volume -DriveLetter $v.DriveLetter -Defrag -ErrorAction Stop; Write-Log ("  {0}: done in {1:N1} min" -f $v.DriveLetter, $sw.Elapsed.TotalMinutes) } catch { Write-Log "  $($v.DriveLetter): $($_.Exception.Message)" 'WARN' }
        }
        Write-Log 'Finished.' 'OK'
    }
    health = {
        foreach ($d in Get-PhysicalDisk) {
            $rel = $d | Get-StorageReliabilityCounter -ErrorAction SilentlyContinue
            Write-Log ("  {0} [{1}] health={2} status={3} temp={4}C wear={5}% powerOnHours={6} readErrors={7}" -f $d.FriendlyName, $d.MediaType, $d.HealthStatus, $d.OperationalStatus, $rel.Temperature, $rel.Wear, $rel.PowerOnHours, $rel.ReadErrorsTotal)
        }
    }
    sfc = {
        $o = (Invoke-Native sfc.exe @('/scannow')) -replace "`0", ''
        $o -split "[\r\n]+" | Where-Object { $_.Trim() -and $_ -notmatch 'Verification \d+%' } | Select-Object -Last 6 | ForEach-Object { Write-Log "  $_" }
    }
    dism = {
        $o = (Invoke-Native Dism.exe @('/Online', '/Cleanup-Image', '/RestoreHealth')) -replace "`0", ''
        $o -split "[\r\n]+" | Where-Object { $_.Trim() -and $_ -notmatch '^\[' } | Select-Object -Last 5 | ForEach-Object { Write-Log "  $_" }
    }
    wu = {
        $svcs = 'wuauserv', 'bits', 'cryptsvc', 'msiserver'
        foreach ($s in $svcs) { Stop-Service $s -Force -ErrorAction SilentlyContinue }
        foreach ($d in "$env:WINDIR\SoftwareDistribution", "$env:WINDIR\System32\catroot2") {
            if (Test-Path "$d.bak") { Remove-Item "$d.bak" -Recurse -Force -ErrorAction SilentlyContinue }
            if (Test-Path $d) { Rename-Item $d "$(Split-Path $d -Leaf).bak" -ErrorAction SilentlyContinue }
        }
        foreach ($s in $svcs) { Start-Service $s -ErrorAction SilentlyContinue }
        Write-Log 'Windows Update components reset. Check for updates again.' 'OK'
    }
    icons = {
        Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 1
        Remove-Item "$env:LOCALAPPDATA\IconCache.db", "$env:LOCALAPPDATA\Microsoft\Windows\Explorer\iconcache_*.db", "$env:LOCALAPPDATA\Microsoft\Windows\Explorer\thumbcache_*.db" -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 1
        if (-not (Get-Process explorer -ErrorAction SilentlyContinue)) { Start-Process explorer.exe }
        Write-Log 'Icon cache rebuilt.' 'OK'
    }
    perfctr = {
        lodctr.exe /r 2>&1 | Out-Null
        lodctr.exe /r 2>&1 | Out-Null
        winmgmt.exe /resyncperf 2>&1 | Out-Null
        Write-Log 'Performance counters rebuilt. Restart apps that read them (Task Manager, monitoring tools).' 'OK'
    }
    regbackup = {
        $dir = Join-Path $OptiBackupDir ('registry-{0}' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
        New-Item -ItemType Directory -Path $dir -Force | Out-Null
        $keys = 'HKLM\SOFTWARE\Policies', 'HKCU\SOFTWARE\Policies', 'HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer', 'HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\ContentDeliveryManager',
                'HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management', 'HKLM\SYSTEM\CurrentControlSet\Control\PriorityControl', 'HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\Parameters'
        foreach ($k in $keys) { $f = Join-Path $dir (($k -replace '[\\ ]', '_') + '.reg'); reg.exe export $k $f /y 2>&1 | Out-Null; Write-Log "  exported $k" }
        Write-Log "Registry backup saved to $dir" 'OK'
    }
    store = { Start-Process wsreset.exe -Wait; Write-Log 'Store cache reset.' 'OK' }
    revert = {
        $state = Get-OptiState
        foreach ($id in @($state.Keys)) { $t = Get-TweakById $id; if ($t) { [void](Invoke-TweakUndo $t) } }
        Write-Log "Reverted $($state.Count) tweak(s)." 'OK'
    }
}

foreach ($k in $global:Tools.Keys) {
    $tool = $global:Tools[$k]
    $b = New-Object Windows.Controls.Button
    $b.Width = 250; $b.Height = 88; $b.Margin = '0,0,8,8'; $b.Padding = '14,0'; $b.HorizontalContentAlignment = 'Left'
    if ($tool.Danger) { $b.Style = $win.FindResource('BtnDanger') }
    $sp = New-Object Windows.Controls.StackPanel
    $tt1 = New-Object Windows.Controls.TextBlock -Property @{ Text = $tool.Title; FontWeight = 'SemiBold'; FontSize = 13 }
    Set-Res $tt1 ([Windows.Controls.TextBlock]::ForegroundProperty) $(if ($tool.Danger) { 'Bad' } else { 'Text' })
    $tt2 = New-Object Windows.Controls.TextBlock -Property @{ Text = $tool.Desc; TextWrapping = 'Wrap'; FontSize = 11; Margin = '0,4,0,0' }
    Set-Res $tt2 ([Windows.Controls.TextBlock]::ForegroundProperty) 'Muted'
    [void]$sp.Children.Add($tt1); [void]$sp.Children.Add($tt2)
    $b.Content = $sp
    $b.Tag = $k
    $b.Add_Click({ param($s, $e) Invoke-Tool $s.Tag })
    [void]$UI.ToolsPanel.Children.Add($b)
}

function Invoke-Tool {
    param([string]$Key)
    if ($Key -eq 'rstrui') { Start-Process rstrui.exe; return }
    if (-not (Test-Elevated)) { return }
    if ($Key -eq 'defrag' -and -not (Confirm-Action 'Defragmenting hard disks can take an hour or more and the app stays busy until it finishes. Continue?')) { return }
    if ($Key -eq 'revert' -and -not (Confirm-Action 'Restore ALL settings changed by Optimaxer to their original values?')) { return }
    $after = if ($Key -eq 'revert') { { Update-TweakStatus } } else { $null }
    [void](Start-OptiJob $global:Tools[$Key].Title -Script $toolJobs[$Key] -OnDone $after)
}

# ================================================================ wiring
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$UI.AdminText.Text = if ($isAdmin) { 'Administrator' } else { 'Not elevated' }
Set-Res $UI.AdminText ([Windows.Controls.TextBlock]::ForegroundProperty) $(if ($isAdmin) { 'Good' } else { 'Bad' })

$UI.BtnMin.Add_Click({ $global:Win.WindowState = 'Minimized' })
$UI.BtnMax.Add_Click({ $global:Win.WindowState = if ($global:Win.WindowState -eq 'Maximized') { 'Normal' } else { 'Maximized' } })
$UI.BtnClose.Add_Click({ $global:Win.Close() })
$win.Add_StateChanged({ $UI.RootGrid.Margin = if ($global:Win.WindowState -eq 'Maximized') { '7' } else { '0' } })
$UI.TileBar.Add_MouseLeftButtonDown({
    param($s, $e)
    $o = $e.OriginalSource
    while ($o -and $o -ne $UI.TileBar) { if ($o -is [Windows.Controls.Primitives.ButtonBase]) { return }; $o = if ($o -is [Windows.Media.Visual]) { [Windows.Media.VisualTreeHelper]::GetParent($o) } else { $null } }
    if ($e.ClickCount -eq 2) { $global:Win.WindowState = if ($global:Win.WindowState -eq 'Maximized') { 'Normal' } else { 'Maximized' } }
    else { $global:Win.DragMove() }
})

foreach ($n in 'NavDash', 'NavTweaks', 'NavServices', 'NavStartup', 'NavCleanup', 'NavDebloat', 'NavNetwork', 'NavFeatures', 'NavTools', 'NavRice') {
    $UI[$n].Add_Checked({ param($s, $e) Show-Page $s.Tag })
}

$UI.BtnOptimize.Add_Click({
    if (-not $global:Loaded['tweaks']) { Write-Log 'Still analyzing, try again in a moment.' 'WARN'; return }
    Set-Preset 'safe'
    $UI.NavTweaks.IsChecked = $true
    Invoke-TweakJob 'apply'
})
$UI.BtnFreeRam.Add_Click({ Invoke-Tool 'ram' })
$UI.BtnRestorePoint.Add_Click({ Invoke-Tool 'restore' })
$UI.BtnQuickClean.Add_Click({ $UI.NavCleanup.IsChecked = $true; Start-CleanScan })

$UI.BtnPrRec.Add_Click({ Set-Preset 'safe' })
$UI.BtnPrGame.Add_Click({ Set-Preset 'gaming' })
$UI.BtnPrPriv.Add_Click({ Set-Preset 'privacy' })
$UI.BtnPrDeai.Add_Click({ Set-Preset 'deai' })
$UI.BtnPrMax.Add_Click({ Set-Preset 'max' })
$UI.BtnPrNone.Add_Click({ Set-Preset 'none' })
$UI.BtnTwApply.Add_Click({ Invoke-TweakJob 'apply' })
$UI.BtnTwUndo.Add_Click({ Invoke-TweakJob 'undo' })
$UI.BtnTwRefresh.Add_Click({ Update-TweakStatus })

$UI.BtnSvRec.Add_Click({ foreach ($r in $global:SvRows) { $r.IsChecked = ($r.Tag -eq 3) } })
$UI.BtnSvManual.Add_Click({ Set-ServiceStartJob 3 'Manual' })
$UI.BtnSvDisable.Add_Click({ Set-ServiceStartJob 4 'Disabled' })
$UI.BtnSvAuto.Add_Click({ Set-ServiceStartJob 2 'Automatic' })
$UI.BtnSvRefresh.Add_Click({ Update-Services })
$UI.BtnSvRestore.Add_Click({
    $f = Get-ChildItem "$global:OptiBackupDir\services-*.json" -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1
    if (-not $f) { Write-Log 'No service backup found.' 'WARN'; return }
    if (-not (Confirm-Action "Restore services from backup $($f.Name)?")) { return }
    [void](Start-OptiJob 'Restoring services' -ArgList $f.FullName -Script {
        param($file)
        foreach ($b in @(Get-Content -LiteralPath $file -Raw | ConvertFrom-Json)) { [void](Set-SvcStart $b.Name $b.Start $b.Delayed); Write-Log "  restored $($b.Name)" }
    } -OnDone { Update-Services })
})

$UI.BtnStDisable.Add_Click({ Set-StartupJob $false })
$UI.BtnStEnable.Add_Click({ Set-StartupJob $true })
$UI.BtnStRefresh.Add_Click({ Update-Startup })

$UI.BtnClScan.Add_Click({ Start-CleanScan })
$UI.BtnClClean.Add_Click({ Start-Clean })
$UI.BtnDbScan.Add_Click({ Start-AppScan })
$UI.BtnDbRemove.Add_Click({ Start-AppRemove })

$UI.BtnFtEnable.Add_Click({ Set-FeatureJob $true })
$UI.BtnFtDisable.Add_Click({ Set-FeatureJob $false })
$UI.BtnFtRefresh.Add_Click({ Update-Features })

$UI.BtnFlushDns.Add_Click({ [void](Start-OptiJob 'Flush DNS' -Script { Clear-DnsClientCache; Write-Log 'DNS cache flushed.' 'OK' }) })
$UI.BtnDnsShow.Add_Click({ Show-CurrentDns })
$UI.BtnResetNet.Add_Click({
    if (-not (Test-Elevated)) { return }
    if (-not (Confirm-Action "Reset Winsock and the IP stack?`nYou may briefly lose connection and need to restart.")) { return }
    [void](Start-OptiJob 'Reset network stack' -Script {
        netsh winsock reset | Out-Null; netsh int ip reset | Out-Null; ipconfig /flushdns | Out-Null
        Write-Log 'Network stack reset. Restart Windows to finish.' 'OK'
    })
})
$UI.BtnDnsBench.Add_Click({
    [void](Start-OptiJob 'Benchmarking DNS servers' -Script {
        $best = $null
        foreach ($p in $OptiDns) {
            if (-not $p.V4.Count) { continue }
            $r = Test-Connection -ComputerName $p.V4[0] -Count 4 -ErrorAction SilentlyContinue
            $avg = if ($r) { [math]::Round(($r | Measure-Object ResponseTime -Average).Average, 1) } else { $null }
            Write-Log ("  {0,-12} {1}" -f $p.Name, $(if ($null -ne $avg) { "$avg ms" } else { 'no reply' }))
            if ($null -ne $avg -and (-not $best -or $avg -lt $best.Avg)) { $best = @{ Name = $p.Name; Avg = $avg } }
        }
        if ($best) { Write-Log "Fastest from your connection: $($best.Name) ($($best.Avg) ms)" 'OK' }
    })
})

$UI.BtnLogClear.Add_Click({ $UI.Log.Clear() })
$UI.BtnLogCopy.Add_Click({ if ($UI.Log.Text) { [Windows.Clipboard]::SetText($UI.Log.Text) } })
$UI.BtnLogOpen.Add_Click({ if (Test-Path $global:OptiLogPath) { Start-Process notepad.exe $global:OptiLogPath } })

# nav tags carry the page name
foreach ($n in 'NavDash', 'NavTweaks', 'NavServices', 'NavStartup', 'NavCleanup', 'NavDebloat', 'NavNetwork', 'NavFeatures', 'NavTools') { }

$timer = New-Object Windows.Threading.DispatcherTimer
$timer.Interval = [TimeSpan]::FromMilliseconds(150)
$tick = 0
$timer.Add_Tick({
    Update-LogPane
    Complete-OptiJobs
    $script:tick++
    if ($script:tick % 10 -eq 0) { Update-Live }
})
$timer.Start()

$win.Add_ContentRendered({
    if (-not $global:Started) {
        $global:Started = $true
        Update-Live
        Start-WindowIn
        if (-not $isAdmin) { Write-Log 'Not running as administrator: most changes will fail. Restart via Launch.bat.' 'WARN' }
        Write-Log "Optimaxer ready. Log file: $global:OptiLogPath"
        Initialize-Dashboard
    }
})

Initialize-Look
Update-Rice
