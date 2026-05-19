# Start all 4 ensemble strategies in separate PowerShell windows
$configs = @(
    "configs/btc_ensemble.env",
    "configs/eth_ensemble.env",
    "configs/btc_15m_ensemble.env",
    "configs/eth_15m_ensemble.env"
)

$root = Split-Path -Parent $MyInvocation.MyCommand.Path

foreach ($cfg in $configs) {
    $title = ($cfg -replace "configs/", "" -replace "\.env", "")
    $cmd = "`$env:STRATEGY_CONFIG='$cfg'; `$Host.UI.RawUI.WindowTitle='$title'; cargo run"
    Start-Process powershell -ArgumentList "-NoExit", "-Command", $cmd -WorkingDirectory $root
    Start-Sleep -Milliseconds 500
}

Write-Host "Started $($configs.Count) strategies."
