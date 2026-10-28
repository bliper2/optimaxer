# Hardware-aware tweaks. Tagged 'smart': the auto-detect plan picks them only when the machine fits (-Need / -MinRamGB / -SSDOnly).
$P = 'Performance'

T -Id 'perf-ntfs-memory' -Retired 'NtfsMemoryUsage is ignored by the dynamic paged pool of 64-bit Windows 10 and 11.' -Cat $P -Name 'NTFS: larger file-system cache pool (16 GB+ RAM)' -Tags max, smart -MinRamGB 15.5 `
  -Desc 'NtfsMemoryUsage = 2 lets NTFS keep more metadata in non-paged memory. Faster file listing and builds on busy drives; costs a little RAM.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' 'NtfsMemoryUsage' 2 ) -Restart

T -Id 'perf-mftzone' -Retired 'Windows already grows the MFT zone as needed; no measurable gain.' -Cat $P -Name 'NTFS: bigger MFT zone on hard disks' -Tags max, smart -Need hdd `
  -Desc 'NtfsMftZoneReservation = 2 reserves 25% of the volume for the file table so it fragments less. Helps HDDs with many small files; ignored on SSDs.' `
  -Reg @( Rg 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' 'NtfsMftZoneReservation' 2 ) -Restart

T -Id 'perf-trimon' -Cat $P -Name 'Make sure NTFS TRIM is enabled (SSD)' -Tags safe, smart -SSDOnly `
  -Desc 'Windows normally enables TRIM on NTFS SSDs; this turns it back on if a tool or driver switched it off. Keeps write speed up over time.' `
  -Snapshot {
      $o = Invoke-Native fsutil.exe @('behavior', 'query', 'DisableDeleteNotify', 'NTFS')
      if ($global:OptiExit -ne 0 -or $o -notmatch 'NTFS\s+DisableDeleteNotify\s*=\s*([01])') { throw 'Could not read the NTFS TRIM setting.' }
      @{ Prev = [int]$Matches[1] }
  } `
  -Apply {
      param($d)
      $o = Invoke-Native fsutil.exe @('behavior', 'set', 'DisableDeleteNotify', 'NTFS', '0')
      if ($global:OptiExit -ne 0) { throw "Could not enable NTFS TRIM: $o" }
  } `
  -Undo {
      param($d)
      if (-not $d -or $null -eq $d.Prev -or $d.Prev -notin 0, 1) { throw 'The saved NTFS TRIM setting is missing or invalid.' }
      $o = Invoke-Native fsutil.exe @('behavior', 'set', 'DisableDeleteNotify', 'NTFS', [string]$d.Prev)
      if ($global:OptiExit -ne 0) { throw "Could not restore NTFS TRIM: $o" }
  } `
  -Test {
      $o = Invoke-Native fsutil.exe @('behavior', 'query', 'DisableDeleteNotify', 'NTFS')
      $global:OptiExit -eq 0 -and $o -match 'NTFS\s+DisableDeleteNotify\s*=\s*0'
  }
