function Write-MediaSiftLog {
    param (
        [string]$Message,
        [string]$Level = "INFO",
        [string]$LogPath
    )

    $timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    $formattedMsg = "[$timestamp] [$Level] $Message"
    Write-Information $formattedMsg -InformationAction Continue

    if ($LogPath -and -not $global:WhatIfPreference) {
        try {
            [System.IO.File]::AppendAllText($LogPath, "$formattedMsg`r`n")
        } catch {
            Write-Warning "Could not append to log file '$LogPath': $_"
        }
    }
}
