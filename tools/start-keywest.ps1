# KeyWest renderer/performance profile adapted from Neptune VE 2026-11-25c.
# GPL-3.0-or-later. See docs/neptune-integration.md for provenance and limits.
[CmdletBinding()]
param(
    [string]$Game,
    [string]$Runtime,
    [ValidateSet('auto', 'series-s', 'series-x')][string]$Console = 'auto',
    [string]$FsrDll,
    [ValidateSet(8, 12, 16, 24)][int]$VramGB = 16,
    [switch]$Warm,
    [switch]$AutoAdvance,
    [switch]$FrametimeDump,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
# Windows PowerShell may evaluate parameter defaults before PSScriptRoot is set.
if (-not $Runtime) {
    $Runtime = Join-Path $PSScriptRoot '..\target\release\victory.exe'
}
if (-not $Game) {
    $gameRoot = Join-Path ([Environment]::GetFolderPath('MyDocuments')) 'GTA6'
    $candidates = @(Get-ChildItem -LiteralPath $gameRoot -Filter 'game_scarlett_master.exe' -File -Recurse -Depth 2 -ErrorAction SilentlyContinue)
    if ($candidates.Count -gt 1) { throw 'More than one game installation found. Pass -Game with the executable to use.' }
    $Game = if ($candidates.Count -eq 1) { $candidates[0].FullName } else { Join-Path $gameRoot 'game_scarlett_master.exe' }
}
if (-not $FsrDll -and $Console -ne 'series-x') {
    $cachedDll = Join-Path $env:LOCALAPPDATA 'victory\dependencies\fidelityfx\amd_fidelityfx_upscaler_dx12.dll'
    if (Test-Path -LiteralPath $cachedDll -PathType Leaf) { $FsrDll = $cachedDll }
}
if (-not $DryRun) {
    if (-not (Test-Path -LiteralPath $Game -PathType Leaf)) { throw "Game executable not found: $Game. Pass -Game with its full path." }
    if (-not (Test-Path -LiteralPath $Runtime -PathType Leaf)) { throw "Runtime not found: $Runtime. Build with: cargo build --release -p victory" }
}
if ($FsrDll -and -not (Test-Path -LiteralPath $FsrDll -PathType Leaf)) { throw "FidelityFX DLL not found: $FsrDll" }
if ($Console -eq 'auto') { $Console = if ($FsrDll) { 'series-s' } else { 'series-x' } }
if ($Console -eq 'series-s' -and -not $FsrDll) {
    throw 'Series S needs -FsrDll pointing to your PC amd_fidelityfx_upscaler_dx12.dll. Use -Console series-x otherwise.'
}
$settings = [ordered]@{
    VICTORY_GAME_DIR = (Split-Path -Parent $Game)
    VICTORY_D3D12_ADAPTER = 'auto'
    VICTORY_CARD_WINDOWS_MB = $(if ($VramGB -ge 16) { '4096' } elseif ($VramGB -ge 12) { '2048' } else { '1024' })
    VICTORY_CARD_BUFFERS_MB = $(if ($VramGB -ge 16) { '1536' } else { '768' })
    VICTORY_TEXTURE_STAGE_MB = $(if ($VramGB -ge 16) { '512' } else { '256' })
    VICTORY_BAKE_TEXTURES_MS = '10000'
    VICTORY_EARLY_TEXTURES_MS = '40'
    VICTORY_WAVE_OWN_LDS_GROUPS = '64,128'
    VICTORY_WIDE_MERGE = '1'
    VICTORY_RELAY_OWN_QUEUE = '1'
    VICTORY_UNDER_WRITES_WORDS = '1'
    VICTORY_UNDER_WRITES_RUNS = '100000'
    VICTORY_UNDER_WRITES_MASKS = '1'
    VICTORY_UNDER_WRITES_MASKS_LEAST = '1'
    VICTORY_RELAY = '2'
    VICTORY_WIDE_REFRESH = '1'
    VICTORY_ASK_ONCE = '1'
    VICTORY_TEXTURE_NO_WAIT = '1'
    VICTORY_CARD_REHASH_RUNS = '1'
    VICTORY_JOIN_COPIES = '1'
    VICTORY_D3D12_BACKGROUND = '0'
    VICTORY_LATE_WAIT_LISTS = '1024'
    VICTORY_WINDOW_TAILS = '1'
    VICTORY_CARD_BUFFERS_MIN_KB = '64'
    VICTORY_ASK_QUIET = '8'
    VICTORY_COPY_CHANGED = '1'
    VICTORY_QUIET_BACK = '1'
    VICTORY_LIST_POOL_MOST = '1024'
    VICTORY_LIST_POOL_MB = '512'
    VICTORY_WIDE_THREADS = ([Math]::Min(12, [Environment]::ProcessorCount)).ToString()
    VICTORY_INDIRECT_RAW = '2'
    VICTORY_TEXTURE_HASH_EVERY = '32'
    VICTORY_LEVELS_ON_DEMAND = '1'
    VICTORY_WINDOW_KB = '1024'
    VICTORY_CARD_WINDOW_UPLOAD = '1'
    VICTORY_DROP_DXIL = '1'
    VICTORY_DESC_DATA = '1'
    VICTORY_VRAM_IDLE_MS = '15000,2500,1000'
    VICTORY_PIPELINE_WAIT_MS = '2500'
    VICTORY_AUTO_A = $(if ($AutoAdvance) { '150' } else { '0' })
    VICTORY_FRAMETIME_DUMP = $(if ($FrametimeDump) { '1' } else { $null })
    VICTORY_FSR_DLL = $(if ($FsrDll) { (Resolve-Path -LiteralPath $FsrDll).Path } else { $null })
}
if ($DryRun) {
    [pscustomobject]@{ Runtime = $Runtime; Game = $Game; Console = $Console; Settings = $settings }
    return
}
# Restore the caller's environment even if warm-up or the title fails.
$previous = @{}
try {
    foreach ($name in $settings.Keys) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        [Environment]::SetEnvironmentVariable($name, $settings[$name], 'Process')
    }
    Write-Host "KeyWest: $Console profile, $VramGB GB VRAM budget."
    if ($Warm) {
        # Avoid reading every texture/audio archive just to find shaders.
        $shaderPack = Join-Path (Split-Path -Parent $Game) 'shaders.rpf'
        if (Test-Path -LiteralPath $shaderPack -PathType Leaf) {
            $inputs = @($Game, $shaderPack)
            $builder = Join-Path (Split-Path -Parent $Game) 'XboxBvhBuilder.dll'
            if (Test-Path -LiteralPath $builder -PathType Leaf) { $inputs += $builder }
            & $Runtime warm @inputs --jobs $settings.VICTORY_WIDE_THREADS
        } else {
            & $Runtime warm --jobs $settings.VICTORY_WIDE_THREADS
        }
        if ($LASTEXITCODE -ne 0) { throw "Shader warm-up failed ($LASTEXITCODE)." }
    }
    & $Runtime run $Game --execute --console $Console
    if ($LASTEXITCODE -ne 0) { throw "KeyWest exited with code $LASTEXITCODE." }
} finally {
    foreach ($name in $previous.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process')
    }
}
