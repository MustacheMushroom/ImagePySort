<#
.SYNOPSIS
    Renames files by prefixing their Creation Date in 'yyyy-MM-dd - ' format.

.DESCRIPTION
    Rename-FilesWithCreationDate inspects files in a target directory and renames any file
    that does not already start with a 'yyyy-MM-dd - ' date prefix. It uses the file system
    CreationTime (or optionally the oldest date between CreationTime and LastWriteTime).

.PARAMETER Path
    Target directory to process. Defaults to current directory.

.PARAMETER Recurse
    If specified, processes subdirectories recursively.

.PARAMETER UseOldestDate
    If specified, compares CreationTime and LastWriteTime and uses whichever timestamp is earlier.
    This is especially helpful for restored/recovered files.

.PARAMETER LogFile
    Path to write execution log. Defaults to 'rename_files.log' in target directory.

.EXAMPLE
    .\Rename-FilesWithCreationDate.ps1 -Path "C:\Photos" -Recurse -WhatIf
    Previews file renaming on C:\Photos without altering any files.

.EXAMPLE
    .\Rename-FilesWithCreationDate.ps1 -Path "C:\Photos" -Recurse
    Executes file renaming on C:\Photos.
#>

[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'Medium')]
param (
    [Parameter(Position = 0, Mandatory = $false)]
    [string]$Path = ".",

    [Parameter(Mandatory = $false)]
    [switch]$Recurse,

    [Parameter(Mandatory = $false)]
    [switch]$UseOldestDate,

    [Parameter(Mandatory = $false)]
    [string]$LogFile = "rename_files.log"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

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

function Get-UniqueNewPath {
    param (
        [string]$DirectoryPath,
        [string]$DesiredName
    )
    $baseName = [System.IO.Path]::GetFileNameWithoutExtension($DesiredName)
    $ext = [System.IO.Path]::GetExtension($DesiredName)
    $destPath = Join-Path -Path $DirectoryPath -ChildPath $DesiredName

    $counter = 1
    while (Test-Path -Path $destPath) {
        $newName = "${baseName}_${counter}${ext}"
        $destPath = Join-Path -Path $DirectoryPath -ChildPath $newName
        $counter++
    }
    return $destPath
}

# Resolve root path
$resolvedPath = Convert-Path -Path $Path
if (-not (Test-Path -Path $resolvedPath -PathType Container)) {
    throw "Target path '$resolvedPath' does not exist or is not a directory."
}

$logFullPath = [System.IO.Path]::Combine($resolvedPath, $LogFile)

Write-MediaSiftLog "Starting creation date rename scan in: $resolvedPath" -LogPath $logFullPath
if ($PSBoundParameters.ContainsKey('WhatIf')) {
    Write-MediaSiftLog "RUNNING IN WHAT-IF (DRY RUN) MODE. No files will be renamed." -Level "WARN" -LogPath $logFullPath
}

# Regex pattern for files that ALREADY have yyyy-MM-dd - prefix
$datePrefixRegex = '^\d{4}-\d{2}-\d{2}\s*-\s*'

# Fetch candidate files
$allFiles = @(Get-ChildItem -Path $resolvedPath -File -Recurse:$Recurse.IsPresent | Where-Object {
    $_.FullName -ne $logFullPath -and
    $_.Name -notmatch $datePrefixRegex
})

Write-MediaSiftLog "Found $($allFiles.Count) candidate files to process." -LogPath $logFullPath

$renamedCount = 0
foreach ($file in $allFiles) {
    # Determine date timestamp
    $targetDate = $file.CreationTime
    if ($UseOldestDate.IsPresent -and $file.LastWriteTime -lt $file.CreationTime) {
        $targetDate = $file.LastWriteTime
    }

    $dateString = $targetDate.ToString("yyyy-MM-dd")
    $newName = "$dateString - $($file.Name)"
    $dirPath = [System.IO.Path]::GetDirectoryName($file.FullName)
    $targetPath = Get-UniqueNewPath -DirectoryPath $dirPath -DesiredName $newName

    if ($psCmdlet.ShouldProcess($file.FullName, "Rename to $([System.IO.Path]::GetFileName($targetPath))")) {
        Rename-Item -Path $file.FullName -NewName ([System.IO.Path]::GetFileName($targetPath))
        Write-MediaSiftLog "Renamed: '$($file.Name)' -> '$([System.IO.Path]::GetFileName($targetPath))'" -LogPath $logFullPath
        $renamedCount++
    }
}

Write-MediaSiftLog "Rename process completed. Renamed: $renamedCount file(s)." -LogPath $logFullPath
