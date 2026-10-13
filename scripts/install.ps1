#Requires -Version 5

# Set strict error handling mode
$ErrorActionPreference = "Stop"  # Make PowerShell throw on all errors

# Colors replaced with Write-Host parameters
$RED = "Red"
$GREEN = "Green"
$YELLOW = "Yellow"
$BLUE = "Blue"
$CYAN = "Cyan"
$LOG_PATH = Join-Path $env:TEMP "bmm-install-$(Get-Date -Format 'yyyyMMddHHmmss').log"

# Clean up function to ensure we always clean temp directory
function Cleanup {
    param([string]$Directory)
    if ($Directory -and (Test-Path $Directory)) {
        Write-Host "Cleaning up build directory..." -ForegroundColor $YELLOW
        Remove-Item $Directory -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# Run a command, stream output, and persist it to a log file for debugging
function Invoke-Step {
    param(
        [string]$Message,
        [scriptblock]$Command
    )

    Write-Host $Message -ForegroundColor $YELLOW
    Add-Content -Path $LOG_PATH -Value "`n[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] $Message"

    & $Command 2>&1 | Tee-Object -FilePath $LOG_PATH -Append
    $exitCode = $LASTEXITCODE

    if ($exitCode -ne 0) {
        $logTail = ""
        if (Test-Path $LOG_PATH) {
            $logTail = (Get-Content $LOG_PATH -Tail 40) -join "`n"
        }
        throw "$Message failed with exit code $exitCode.`n$logTail`nFull log: $LOG_PATH"
    }
}

# Handle script interruption
$BUILD_DIR = $null
trap {
    Write-Host "Script interrupted or error encountered" -ForegroundColor $RED
    Write-Host $_ -ForegroundColor $RED
    if ($BUILD_DIR) { 
        Cleanup -Directory $BUILD_DIR 
    }
    exit 1
}

Write-Host @"
    ____  __  _____  ___            ____           __        ____
   / __ )/  |/  /  |/  /           /  _/___  _____/ /_____ _/ / /
  / __  / /|_/ / /|_/ /  ______    / // __ \/ ___/ __/ __ `/ / /
 / /_/ / /  / / /  / /  /_____/  _/ // / / (__  ) /_/ /_/ / / /
/_____/_/  /_/_/  /_/           /___/_/ /_/____/\__/\__,_/_/_/
"@ -ForegroundColor $CYAN

Write-Host "Balatro Mod Manager Builder" -ForegroundColor $GREEN
Write-Host "----------------------------------------"
Write-Host "Build started at $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
Write-Host "Build log: $LOG_PATH" -ForegroundColor $YELLOW

# OS check
if ($env:OS -ne "Windows_NT") {
    Write-Host "Error: This builder is for Windows only." -ForegroundColor $RED
    exit 1
}

# Dependency checks
$deps = @(
    @{Name="git"; Url="https://git-scm.com/downloads"},
    @{Name="cargo"; Url="https://www.rust-lang.org/tools/install"},
    @{Name="bun"; Url="https://bun.sh/"},
    @{Name="cargo-tauri"; Url="https://crates.io/crates/tauri-cli"}
)

Write-Host "Checking dependencies..." -ForegroundColor $YELLOW
foreach ($dep in $deps) {
    if (-not (Get-Command $dep.Name -ErrorAction SilentlyContinue)) {
        Write-Host "Error: $($dep.Name) not found. Please install first." -ForegroundColor $RED
        Write-Host $dep.Url -ForegroundColor $BLUE
        exit 1
    }
}

# Check cargo-tauri version
Write-Host "Checking Tauri CLI version..." -ForegroundColor $YELLOW
try {
    $tauriVersionOutput = (cargo tauri --version) -join ""
    if ($tauriVersionOutput -match '(\d+\.\d+\.\d+)') {
        $TAURI_VERSION = $matches[1]
        $REQUIRED_VERSION = "2.9.0"
        
        # Convert versions to System.Version for proper comparison
        $currentVersion = [System.Version]$TAURI_VERSION
        $requiredVersion = [System.Version]$REQUIRED_VERSION
        
        if ($currentVersion -lt $requiredVersion) {
            Write-Host "Error: cargo-tauri version $TAURI_VERSION is too old. Please update to at least version $REQUIRED_VERSION" -ForegroundColor $RED
            exit 1
        }
        Write-Host "cargo-tauri version $TAURI_VERSION ✓" -ForegroundColor $GREEN
    } else {
        Write-Host "Error: Unable to determine cargo-tauri version" -ForegroundColor $RED
        exit 1
    }
} catch {
    Write-Host "Error checking cargo-tauri version: $_" -ForegroundColor $RED
    exit 1
}

# Check bun version (older Bun builds can break Vite)
try {
    $bunVersionOutput = (bun --version) -join ""
    if ($bunVersionOutput -match '(\d+\.\d+\.\d+)') {
        $BUN_VERSION = $matches[1]
        $REQUIRED_BUN = "1.0.0"
        $currentBun = [System.Version]$BUN_VERSION
        $requiredBun = [System.Version]$REQUIRED_BUN

        if ($currentBun -lt $requiredBun) {
            Write-Host "Error: bun version $BUN_VERSION is too old. Please update to at least version $REQUIRED_BUN" -ForegroundColor $RED
            exit 1
        }
        Write-Host "bun version $BUN_VERSION ✓" -ForegroundColor $GREEN
    } else {
        Write-Host "Warning: Unable to determine bun version. Continuing anyway." -ForegroundColor $YELLOW
    }
} catch {
    Write-Host "Error checking bun version: $_" -ForegroundColor $RED
    exit 1
}

# Create temp directory
$BUILD_DIR = Join-Path $env:TEMP "balatro-mod-manager-$(Get-Date -Format 'yyyyMMddHHmmss')"
Write-Host "Creating temporary build directory: ${BUILD_DIR}" -ForegroundColor $YELLOW
New-Item -Path $BUILD_DIR -ItemType Directory -Force | Out-Null

# Clone repository
try {
    # Shallow clone keeps downloads small and avoids long paths in temp dirs
    $clonePath = Join-Path $BUILD_DIR "balatro-mod-manager"
    Invoke-Step "1. Cloning repository..." { git clone --depth 1 https://github.com/rursache/balatro-mod-manager.git $clonePath }
} catch {
    Write-Host "Error during repository cloning: $_" -ForegroundColor $RED
    Cleanup -Directory $BUILD_DIR
    exit 1
}

# Build process
try {
    # Record original location to return to it after build
    $originalLocation = Get-Location
    
    Set-Location (Join-Path $BUILD_DIR "balatro-mod-manager")
    
    Invoke-Step "2. Installing bun dependencies..." { bun install }
    Invoke-Step "3. Building frontend..." { bun run build }

    Push-Location src-tauri
    $env:SKIP_BUILD_SCRIPT = "1"
    Invoke-Step "4. Building Rust backend..." { cargo build --release }
    Pop-Location

    Invoke-Step "5. Creating app bundle..." { cargo tauri build }
    
    # Return to original location
    Set-Location $originalLocation
    
    Write-Host "Installation completed successfully!" -ForegroundColor $GREEN
    Write-Host ""
    Write-Host "Note: Windows SmartScreen might block first execution -`nright-click the .exe and select 'Run anyway'" -ForegroundColor $YELLOW
}
catch {
    Write-Host "Build error: $_" -ForegroundColor $RED
    # Return to original location before cleaning up
    Set-Location $originalLocation
    Cleanup -Directory $BUILD_DIR
    exit 1
}
finally {
    # Make sure we're back at the original location
    if ((Get-Location).Path -ne $originalLocation.Path) {
        Set-Location $originalLocation
    }
}
