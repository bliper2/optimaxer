# Hardware-aware tweaks. Tagged 'smart': the auto-detect plan picks them only when the machine fits (-Need / -MinRamGB / -SSDOnly).
$P = 'Performance'

T -Id 'perf-ntfs-memory' -Cat $P -Name 'NTFS: larger file-system cache pool (16 GB+ RAM)' -Tags max, smart -MinRamGB 15.5 `
  -Desc 'NtfsMemoryUsage = 2 lets NTFS keep more metadata in non-paged memory. Faster file listing and builds on busy drives; costs a little RAM.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' 'NtfsMemoryUsage' 2 ) -Restart

T -Id 'perf-mftzone' -Cat $P -Name 'NTFS: bigger MFT zone on hard disks' -Tags max, smart -Need hdd `
  -Desc 'NtfsMftZoneReservation = 2 reserves 25% of the volume for the file table so it fragments less. Helps HDDs with many small files; ignored on SSDs.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' 'NtfsMftZoneReservation' 2 ) -Restart

T -Id 'perf-trimon' -Cat $P -Name 'Make sure TRIM is enabled (SSD)' -Tags safe, smart -SSDOnly `
  -Desc 'Windows normally enables TRIM on SSDs; this turns it back on if a tool or driver switched it off. Keeps write speed up over time.' `
  -Apply { $o = Invoke-Native fsutil @('behavior', 'query', 'DisableDeleteNotify'); $prev = if ($o -match 'NTFS\s+DisableDeleteNotify\s*=\s*(\d)') { [int]$Matches[1] } else { 0 }; fsutil behavior set DisableDeleteNotify 0 | Out-Null; @{ Prev = $prev } } `
  -Undo  { param($d) if ($d -and $d.Prev -eq 1) { fsutil behavior set DisableDeleteNotify 1 | Out-Null } } `
  -Test  { (Invoke-Native fsutil @('behavior', 'query', 'DisableDeleteNotify')) -match 'NTFS\s+DisableDeleteNotify\s*=\s*0' }
