# Dev-time exporter: reads the Optimaxer PowerShell catalog and writes the data tables the Rust port embeds.
# Read-only: it only dot-sources the catalog (object construction) and serializes it. Nothing is applied.
# Usage: powershell -NoProfile -File tools\export-optimaxer.ps1 -Out assets\optimaxer
param([string]$Optimaxer = (Resolve-Path "$PSScriptRoot\..\..").Path, [string]$Out = "$PSScriptRoot\..\assets\optimaxer")
$ErrorActionPreference = 'Stop'
$global:OptiRoot = $Optimaxer
. "$Optimaxer\src\Core.ps1"; . "$Optimaxer\src\Data.ps1"; . "$Optimaxer\src\Catalog.ps1"
New-Item -ItemType Directory -Path $Out -Force | Out-Null

function Convert-Val($v) {
    if ($v -is [byte[]]) { return ,@($v | ForEach-Object { [int]$_ }) }
    if ($v -is [array]) { return ,@($v | ForEach-Object { [string]$_ }) }
    if ($v -is [string]) { return $v }
    return [int64]$v
}

$tweaks = foreach ($t in $global:OptiCatalog) {
    [ordered]@{
        id = $t.Id; name = $t.Name; desc = $t.Desc; cat = $t.Cat; risk = $t.Risk; tags = @($t.Tags)
        reg  = @($t.Reg  | ForEach-Object { [ordered]@{ p = $_.P; n = $_.N; v = (Convert-Val $_.V); t = $_.T } })
        svc  = @($t.Svc  | ForEach-Object { [ordered]@{ name = $_.Name; start = [int]$_.Start } })
        task = @($t.Task)
        restart = $t.Restart; explorer = $t.Explorer; desktop_only = $t.DesktopOnly; ssd_only = $t.SSDOnly
        vendor = $t.Vendor; svc_tol = $t.SvcTol; min_ram = $t.MinRamGB; need = @($t.Need); conflict = [string]$t.Conflict; retired = [string]$t.Retired
        script = [ordered]@{ apply = [bool]$t.Apply; undo = [bool]$t.Undo; test = [bool]$t.Test; snapshot = [bool]$t.Snapshot }
        apply_text = $(if ($t.Apply) { $t.Apply.ToString() } else { '' })
    }
}
ConvertTo-Json -InputObject @($tweaks) -Depth 8 | Set-Content -LiteralPath (Join-Path $Out 'tweaks.json') -Encoding UTF8

$data = [ordered]@{
    service_recs = @($global:OptiServiceRecs | ForEach-Object { [ordered]@{ name = $_.Name; start = [int]$_.Start; desc = $_.Desc } })
    protected    = @($global:OptiProtectedServices)
    dns          = @($global:OptiDns | ForEach-Object { [ordered]@{ name = $_.Name; desc = $_.Desc; v4 = @($_.V4); v6 = @($_.V6); doh = $_.Doh } })
}
ConvertTo-Json -InputObject $data -Depth 6 | Set-Content -LiteralPath (Join-Path $Out 'data.json') -Encoding UTF8
"exported $(@($tweaks).Count) tweaks"
