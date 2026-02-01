# Create Portable AI-Foundation Package
# Run: .\create-portable-package.ps1
#
# Creates a minimal, portable distribution that "just works"

param(
    [string]$OutputDir = ".\ai-foundation-portable",
    [switch]$IncludeEmbeddings = $true,
    [switch]$Minimal = $false
)

$ErrorActionPreference = "Stop"

Write-Host "Creating AI-Foundation Portable Package..." -ForegroundColor Cyan
Write-Host ""

# Core binaries (always included)
$CoreBinaries = @(
    "notebook-cli.exe",      # Memory/notes
    "teambook.exe",          # Team coordination
    "session-start.exe",     # Session hooks
    "hook-cli.exe"           # Post-tool hooks
)

# Extended binaries (included unless -Minimal)
$ExtendedBinaries = @(
    "benchmark.exe",         # Performance testing
    "shm-bench.exe",         # Shared memory testing
    "hybrid-server.exe",     # Awareness server
    "visionbook.exe",        # Screenshots
    "afp-cli.exe",           # Federation protocol
    "afp-server.exe"
)

# Model files
$Models = @(
    "embeddinggemma-300M-Q8_0.gguf"
)

# Create output directory
if (Test-Path $OutputDir) {
    Remove-Item -Recurse -Force $OutputDir
}
New-Item -ItemType Directory -Path $OutputDir | Out-Null
New-Item -ItemType Directory -Path "$OutputDir\bin" | Out-Null

$SourceBin = ".\bin"
$copied = 0
$totalSize = 0

# Copy core binaries
Write-Host "Copying core binaries..." -ForegroundColor Yellow
foreach ($bin in $CoreBinaries) {
    $src = Join-Path $SourceBin $bin
    if (Test-Path $src) {
        Copy-Item $src "$OutputDir\bin\"
        $size = (Get-Item $src).Length / 1MB
        $totalSize += $size
        Write-Host "  + $bin ($([math]::Round($size, 1)) MB)"
        $copied++
    }
}

# Copy extended binaries (unless minimal)
if (-not $Minimal) {
    Write-Host "Copying extended binaries..." -ForegroundColor Yellow
    foreach ($bin in $ExtendedBinaries) {
        $src = Join-Path $SourceBin $bin
        if (Test-Path $src) {
            Copy-Item $src "$OutputDir\bin\"
            $size = (Get-Item $src).Length / 1MB
            $totalSize += $size
            Write-Host "  + $bin ($([math]::Round($size, 1)) MB)"
            $copied++
        }
    }
}

# Copy model (if embeddings enabled)
if ($IncludeEmbeddings) {
    Write-Host "Copying embedding model..." -ForegroundColor Yellow
    foreach ($model in $Models) {
        $src = Join-Path $SourceBin $model
        if (Test-Path $src) {
            Copy-Item $src "$OutputDir\bin\"
            $size = (Get-Item $src).Length / 1MB
            $totalSize += $size
            Write-Host "  + $model ($([math]::Round($size, 1)) MB)"
        }
    }
}

# Create README
$readme = @"
# AI-Foundation Portable

## Quick Start

1. Set your AI identity:
   ```
   set AI_ID=your-name-123
   ```

2. Save a note:
   ```
   bin\notebook-cli.exe remember "Your first note" --tags hello,test
   ```

3. Search your notes:
   ```
   bin\notebook-cli.exe recall "search query"
   ```

## What's Included

- **notebook-cli.exe** - Personal AI memory (notes, vault, graph)
- **teambook.exe** - Multi-AI coordination (DMs, broadcasts, votes)
- **session-start.exe** - Session initialization hook
- **hook-cli.exe** - Post-tool awareness hook

## Data Location

All data is stored in `.ai-foundation/` in the current directory:
- `notebook.db` - Your notes (SQLite)
- `shm/` - Shared memory IPC region

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| AI_ID | Your AI identity | "unknown" |
| NOTEBOOK_PATH | Custom notebook location | .ai-foundation/notebook.db |
| POSTGRES_URL | Teambook database | (required for teambook) |

## Performance

This build includes:
- **SIMD (AVX-512/AVX2)** - 7x faster vector operations
- **mimalloc** - High-performance allocator
- **Lock-free IPC** - Sub-microsecond messaging

The optimizations auto-detect your CPU and use the best available.

---
Generated: $(Get-Date -Format "yyyy-MM-dd HH:mm:ss")
"@

$readme | Out-File -FilePath "$OutputDir\README.md" -Encoding UTF8

# Create simple batch launcher
$launcher = @"
@echo off
if "%AI_ID%"=="" set AI_ID=my-ai
echo AI-Foundation starting as %AI_ID%
bin\notebook-cli.exe stats
"@
$launcher | Out-File -FilePath "$OutputDir\status.bat" -Encoding ASCII

Write-Host ""
Write-Host "Package created: $OutputDir" -ForegroundColor Green
Write-Host "  Files: $copied binaries"
Write-Host "  Size: $([math]::Round($totalSize, 1)) MB"
Write-Host ""
Write-Host "To distribute: zip the '$OutputDir' folder" -ForegroundColor Cyan
