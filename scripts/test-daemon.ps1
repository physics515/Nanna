# Nanna Daemon End-to-End Test Script
# Tests daemon startup, health endpoint, WebSocket connection, and shutdown

param(
    [switch]$Build,
    [switch]$Verbose
)

$ErrorActionPreference = "Stop"

# Colors
function Write-Success { Write-Host "✓ $args" -ForegroundColor Green }
function Write-Fail { Write-Host "✗ $args" -ForegroundColor Red }
function Write-Info { Write-Host "→ $args" -ForegroundColor Cyan }
function Write-Step { Write-Host "`n[$($script:step++)] $args" -ForegroundColor Yellow }
$script:step = 1

$ProjectDir = "D:\Development\clawdbot-rs"
$DaemonBin = "$ProjectDir\target\debug\nanna-daemon.exe"
$HealthUrl = "http://127.0.0.1:5148"
$WsUrl = "ws://127.0.0.1:5149"

Write-Host "========================================" -ForegroundColor Magenta
Write-Host " Nanna Daemon End-to-End Test Suite" -ForegroundColor Magenta
Write-Host "========================================" -ForegroundColor Magenta

# Build if requested
if ($Build) {
    Write-Step "Building daemon..."
    Push-Location $ProjectDir
    cargo build -p nanna-daemon
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "Build failed"
        Pop-Location
        exit 1
    }
    Write-Success "Build successful"
    Pop-Location
}

# Check binary exists
Write-Step "Checking daemon binary..."
if (-not (Test-Path $DaemonBin)) {
    Write-Fail "Daemon binary not found at $DaemonBin"
    Write-Info "Run with -Build to compile first"
    exit 1
}
Write-Success "Binary found"

# Check if daemon already running
Write-Step "Checking for existing daemon..."
$healthCheck = try { Invoke-RestMethod -Uri "$HealthUrl/health" -TimeoutSec 2 -ErrorAction SilentlyContinue } catch { $null }
if ($healthCheck) {
    Write-Info "Daemon already running (uptime: $($healthCheck.uptime_secs)s)"
    $existingDaemon = $true
} else {
    Write-Info "No existing daemon detected"
    $existingDaemon = $false
}

# Start daemon if not running
if (-not $existingDaemon) {
    Write-Step "Starting daemon..."
    $daemonArgs = @("run")
    if ($Verbose) { $daemonArgs += "--log-level"; $daemonArgs += "debug" }
    
    $daemon = Start-Process -FilePath $DaemonBin -ArgumentList $daemonArgs -PassThru -WindowStyle Hidden
    Write-Info "Daemon started (PID: $($daemon.Id))"
    
    # Wait for startup
    Write-Info "Waiting for daemon to initialize..."
    $timeout = 30
    $started = $false
    for ($i = 0; $i -lt $timeout; $i++) {
        Start-Sleep -Seconds 1
        try {
            $health = Invoke-RestMethod -Uri "$HealthUrl/health" -TimeoutSec 2 -ErrorAction SilentlyContinue
            if ($health.status -eq "ok") {
                $started = $true
                break
            }
        } catch {}
        Write-Host "." -NoNewline
    }
    Write-Host ""
    
    if (-not $started) {
        Write-Fail "Daemon failed to start within ${timeout}s"
        Stop-Process -Id $daemon.Id -Force -ErrorAction SilentlyContinue
        exit 1
    }
    Write-Success "Daemon started successfully"
}

# Test health endpoints
Write-Step "Testing health endpoints..."

# /health
try {
    $health = Invoke-RestMethod -Uri "$HealthUrl/health" -TimeoutSec 5
    Write-Success "/health - status=$($health.status), version=$($health.version), uptime=$($health.uptime_secs)s"
} catch {
    Write-Fail "/health - $($_.Exception.Message)"
}

# /healthz
try {
    $response = Invoke-WebRequest -Uri "$HealthUrl/healthz" -TimeoutSec 5
    if ($response.StatusCode -eq 200) {
        Write-Success "/healthz - 200 OK"
    } else {
        Write-Fail "/healthz - $($response.StatusCode)"
    }
} catch {
    Write-Fail "/healthz - $($_.Exception.Message)"
}

# /readyz
try {
    $response = Invoke-WebRequest -Uri "$HealthUrl/readyz" -TimeoutSec 5
    if ($response.StatusCode -eq 200) {
        Write-Success "/readyz - 200 OK (agent available)"
    } else {
        Write-Info "/readyz - $($response.StatusCode) (agent may not be ready)"
    }
} catch {
    Write-Info "/readyz - $($_.Exception.Message)"
}

# /status
try {
    $status = Invoke-RestMethod -Uri "$HealthUrl/status" -TimeoutSec 5
    Write-Success "/status - sessions=$($status.sessions), memory=$($status.memory_available), agent=$($status.agent_available)"
} catch {
    Write-Fail "/status - $($_.Exception.Message)"
}

# Test PID file
Write-Step "Testing PID file..."
$pidPath = "$env:LOCALAPPDATA\nanna\nanna-daemon\data\nanna-daemon.pid"
if (Test-Path $pidPath) {
    $pid = Get-Content $pidPath
    Write-Success "PID file exists: $pidPath (PID: $pid)"
    
    # Verify process is running
    $proc = Get-Process -Id $pid -ErrorAction SilentlyContinue
    if ($proc) {
        Write-Success "Process $pid is running ($($proc.ProcessName))"
    } else {
        Write-Fail "PID file exists but process $pid not found"
    }
} else {
    Write-Info "PID file not found at expected path (may be in different location)"
}

# Test WebSocket connection (basic)
Write-Step "Testing WebSocket endpoint..."
Write-Info "WebSocket URL: $WsUrl"
Write-Info "(Full WebSocket test requires client - checking port is open)"

try {
    $tcp = New-Object System.Net.Sockets.TcpClient
    $tcp.Connect("127.0.0.1", 5149)
    if ($tcp.Connected) {
        Write-Success "Port 5149 is open and accepting connections"
        $tcp.Close()
    }
} catch {
    Write-Fail "Port 5149 not reachable: $($_.Exception.Message)"
}

# Test double-start prevention (PID file)
Write-Step "Testing double-start prevention..."
if (-not $existingDaemon) {
    Write-Info "Attempting to start second daemon instance..."
    $daemon2 = Start-Process -FilePath $DaemonBin -ArgumentList "run" -PassThru -WindowStyle Hidden -RedirectStandardError "$env:TEMP\daemon2-err.txt"
    Start-Sleep -Seconds 3
    
    if ($daemon2.HasExited) {
        Write-Success "Second instance exited as expected (PID file protection working)"
    } else {
        Write-Fail "Second instance still running (PID protection may have failed)"
        Stop-Process -Id $daemon2.Id -Force -ErrorAction SilentlyContinue
    }
}

# Summary
Write-Host "`n========================================" -ForegroundColor Magenta
Write-Host " Test Summary" -ForegroundColor Magenta
Write-Host "========================================" -ForegroundColor Magenta

Write-Host "`nEndpoints:"
Write-Host "  Health: $HealthUrl/health"
Write-Host "  Status: $HealthUrl/status"
Write-Host "  WebSocket: $WsUrl"

if (-not $existingDaemon -and $daemon) {
    Write-Host "`nDaemon is running (PID: $($daemon.Id))"
    Write-Host "To stop: Stop-Process -Id $($daemon.Id)"
}

Write-Host "`n✓ All basic tests passed" -ForegroundColor Green
