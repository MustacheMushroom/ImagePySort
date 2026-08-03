<#
.SYNOPSIS
    Identifies binary-identical files and safely moves originals and duplicate groups into separate subfolders.

.DESCRIPTION
    Organize-DuplicateFiles scans a target directory for duplicate files using a two-stage
    verification process (file size comparison followed by SHA-256 hashing, with optional byte-by-byte comparison).
    For each set of identical files, one designated original is moved to the 'Originals' directory, and all duplicate
    copies are moved to distinct group subdirectories inside 'Duplicates'.

.PARAMETER Path
    The target directory to scan. Defaults to current directory.

.PARAMETER Recurse
    If specified, scans subdirectories recursively.

.PARAMETER OriginalsDir
    The folder where original copies will be moved. Defaults to 'Originals' within Path.

.PARAMETER DuplicatesDir
    The folder where duplicate groups will be stored. Defaults to 'Duplicates' within Path.

.PARAMETER IncludeUnique
    If specified, unique files (files with no duplicate copies) will also be moved to the Originals directory.

.PARAMETER VerifyByte
    If specified, performs byte-by-byte comparison for files sharing identical SHA-256 hashes to guarantee binary identity.

.PARAMETER LogFile
    Path to write a log file of operations. Defaults to 'organize_duplicates.log' in the target directory.

.EXAMPLE
    .\Organize-DuplicateFiles.ps1 -Path "C:\Data" -Recurse -WhatIf
    Previews operations on C:\Data recursively without moving any files.

.EXAMPLE
    .\Organize-DuplicateFiles.ps1 -Path "C:\Data" -Recurse
    Executes duplicate organizing on C:\Data.
#>

[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'Medium')]
param (
    [Parameter(Position = 0, Mandatory = $false)]
    [string]$Path = ".",

    [Parameter(Mandatory = $false)]
    [switch]$Recurse,

    [Parameter(Mandatory = $false)]
    [string]$OriginalsDir = "Originals",

    [Parameter(Mandatory = $false)]
    [string]$DuplicatesDir = "Duplicates",

    [Parameter(Mandatory = $false)]
    [string]$LeftoversDir = "Leftovers",

    [Parameter(Mandatory = $false)]
    [switch]$MoveLeftovers,

    [Parameter(Mandatory = $false)]
    [switch]$PreserveStructure,

    [Parameter(Mandatory = $false)]
    [switch]$IncludeUnique,

    [Parameter(Mandatory = $false)]
    [switch]$VerifyByte,

    [Parameter(Mandatory = $false)]
    [string]$LogFile = "organize_duplicates.log"
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

function Get-UniqueDestinationPath {
    param (
        [string]$TargetFolder,
        [string]$FileName
    )
    $baseName = [System.IO.Path]::GetFileNameWithoutExtension($FileName)
    $ext = [System.IO.Path]::GetExtension($FileName)
    $destPath = Join-Path -Path $TargetFolder -ChildPath $FileName

    $counter = 1
    while (Test-Path -Path $destPath) {
        $newName = "${baseName}_${counter}${ext}"
        $destPath = Join-Path -Path $TargetFolder -ChildPath $newName
        $counter++
    }
    return $destPath
}

function Get-DestinationPathForFile {
    param (
        [string]$BaseTargetFolder,
        [System.IO.FileInfo]$File,
        [string]$RootPath,
        [bool]$ShouldPreserveStructure
    )
    $targetFolder = $BaseTargetFolder
    if ($ShouldPreserveStructure -and $RootPath -and $File.FullName.StartsWith($RootPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        $relDir = [System.IO.Path]::GetDirectoryName($File.FullName.Substring($RootPath.Length).TrimStart('\', '/'))
        if (-not [string]::IsNullOrWhiteSpace($relDir)) {
            $targetFolder = [System.IO.Path]::Combine($BaseTargetFolder, $relDir)
        }
    }

    if (-not $global:WhatIfPreference -and -not (Test-Path -Path $targetFolder)) {
        New-Item -ItemType Directory -Path $targetFolder -Force | Out-Null
    }

    return Get-UniqueDestinationPath -TargetFolder $targetFolder -FileName $File.Name
}

function Compare-FilesByteByByte {
    param (
        [string]$Path1,
        [string]$Path2
    )
    $file1 = [System.IO.File]::OpenRead($Path1)
    $file2 = [System.IO.File]::OpenRead($Path2)

    try {
        if ($file1.Length -ne $file2.Length) { return $false }

        $buffer1 = New-Object byte[] 65536
        $buffer2 = New-Object byte[] 65536

        while ($true) {
            $count1 = $file1.Read($buffer1, 0, $buffer1.Length)
            $count2 = $file2.Read($buffer2, 0, $buffer2.Length)

            if ($count1 -ne $count2) { return $false }
            if ($count1 -eq 0) { return $true }

            for ($i = 0; $i -lt $count1; $i++) {
                if ($buffer1[$i] -ne $buffer2[$i]) { return $false }
            }
        }
    }
    finally {
        $file1.Close()
        $file2.Close()
    }
}

# Resolve root path
$resolvedPath = Convert-Path -Path $Path
if (-not (Test-Path -Path $resolvedPath -PathType Container)) {
    throw "Target path '$resolvedPath' does not exist or is not a directory."
}

# Resolve output directory full paths
$origFullPath = [System.IO.Path]::Combine($resolvedPath, $OriginalsDir)
$dupFullPath = [System.IO.Path]::Combine($resolvedPath, $DuplicatesDir)
$leftFullPath = [System.IO.Path]::Combine($resolvedPath, $LeftoversDir)
$logFullPath = [System.IO.Path]::Combine($resolvedPath, $LogFile)

Write-MediaSiftLog "Starting duplicate scan in: $resolvedPath" -LogPath $logFullPath
Write-MediaSiftLog "Originals directory: $origFullPath" -LogPath $logFullPath
Write-MediaSiftLog "Duplicates directory: $dupFullPath" -LogPath $logFullPath
if ($MoveLeftovers.IsPresent) {
    Write-MediaSiftLog "Leftovers directory: $leftFullPath" -LogPath $logFullPath
}
if ($PSBoundParameters.ContainsKey('WhatIf')) {
    Write-MediaSiftLog "RUNNING IN WHAT-IF (DRY RUN) MODE. No files will be moved." -Level "WARN" -LogPath $logFullPath
}

# Ensure destination folders exist if not WhatIf
if (-not $WhatIfPreference) {
    if (-not (Test-Path -Path $origFullPath)) {
        New-Item -ItemType Directory -Path $origFullPath -Force | Out-Null
    }
    if (-not (Test-Path -Path $dupFullPath)) {
        New-Item -ItemType Directory -Path $dupFullPath -Force | Out-Null
    }
    if ($MoveLeftovers.IsPresent -and -not (Test-Path -Path $leftFullPath)) {
        New-Item -ItemType Directory -Path $leftFullPath -Force | Out-Null
    }
}

# Scan files, excluding output directories and log file
$allFiles = Get-ChildItem -Path $resolvedPath -File -Recurse:$Recurse.IsPresent | Where-Object {
    $_.FullName -notlike "$origFullPath*" -and
    $_.FullName -notlike "$dupFullPath*" -and
    $_.FullName -notlike "$leftFullPath*" -and
    $_.FullName -ne $logFullPath
}

Write-MediaSiftLog "Found $($allFiles.Count) candidate files to evaluate." -LogPath $logFullPath

# Stage 1: Group by File Size
$sizeGroups = $allFiles | Group-Object -Property Length
$candidateGroups = $sizeGroups | Where-Object { $_.Count -gt 1 }
$uniqueSizeFiles = $sizeGroups | Where-Object { $_.Count -eq 1 }

Write-MediaSiftLog "Found $($candidateGroups.Count) size groups containing potential duplicates." -LogPath $logFullPath

# Handle Unique files if requested
if ($IncludeUnique.IsPresent -or $MoveLeftovers.IsPresent) {
    $targetDir = if ($MoveLeftovers.IsPresent) { $leftFullPath } else { $origFullPath }
    foreach ($u in $uniqueSizeFiles) {
        $file = $u.Group[0]
        $targetPath = Get-DestinationPathForFile -BaseTargetFolder $targetDir -File $file -RootPath $resolvedPath -ShouldPreserveStructure $PreserveStructure.IsPresent
        if ($psCmdlet.ShouldProcess($file.FullName, "Move non-duplicate file to $targetPath")) {
            Move-Item -Path $file.FullName -Destination $targetPath
            Write-MediaSiftLog "Moved non-duplicate file: '$($file.FullName)' -> '$targetPath'" -LogPath $logFullPath
        }
    }
}

# Stage 2: Hash Files with Same Size
$duplicateSets = @()

foreach ($group in $candidateGroups) {
    $hashedFiles = foreach ($file in $group.Group) {
        try {
            $hashObj = Get-FileHash -Path $file.FullName -Algorithm SHA256
            [PSCustomObject]@{
                File = $file
                Hash = $hashObj.Hash
            }
        }
        catch {
            Write-MediaSiftLog "Failed to calculate hash for '$($file.FullName)': $_" -Level "ERROR" -LogPath $logFullPath
            $null
        }
    }

    # Group by Hash
    $hashGroups = $hashedFiles | Where-Object { $_ -ne $null } | Group-Object -Property Hash

    foreach ($hGroup in $hashGroups) {
        if ($hGroup.Count -gt 1) {
            # Optional Stage 3: Byte-by-byte verification
            if ($VerifyByte.IsPresent) {
                $verifiedGroup = @($hGroup.Group[0])
                for ($i = 1; $i -lt $hGroup.Group.Count; $i++) {
                    if (Compare-FilesByteByByte -Path1 $hGroup.Group[0].File.FullName -Path2 $hGroup.Group[$i].File.FullName) {
                        $verifiedGroup += $hGroup.Group[$i]
                    } else {
                        Write-MediaSiftLog "Hash matched but byte comparison failed between '$($hGroup.Group[0].File.FullName)' and '$($hGroup.Group[$i].File.FullName)'" -Level "WARN" -LogPath $logFullPath
                    }
                }
                if ($verifiedGroup.Count -gt 1) {
                    $duplicateSets += , $verifiedGroup
                }
            } else {
                $duplicateSets += , $hGroup.Group
            }
        } elseif ($IncludeUnique.IsPresent -or $MoveLeftovers.IsPresent) {
            $file = $hGroup.Group[0].File
            $targetDir = if ($MoveLeftovers.IsPresent) { $leftFullPath } else { $origFullPath }
            $targetPath = Get-DestinationPathForFile -BaseTargetFolder $targetDir -File $file -RootPath $resolvedPath -ShouldPreserveStructure $PreserveStructure.IsPresent
            if ($psCmdlet.ShouldProcess($file.FullName, "Move non-duplicate file to $targetPath")) {
                Move-Item -Path $file.FullName -Destination $targetPath
                Write-MediaSiftLog "Moved non-duplicate file: '$($file.FullName)' -> '$targetPath'" -LogPath $logFullPath
            }
        }
    }
}

Write-MediaSiftLog "Identified $($duplicateSets.Count) set(s) of binary identical files." -LogPath $logFullPath

# Process Duplicate Sets
$groupCounter = 1

foreach ($set in $duplicateSets) {
    $shortHash = $set[0].Hash.Substring(0, 8)
    $groupFolderName = "Group_$("{0:D3}" -f $groupCounter)_$shortHash"
    $groupFolderPath = Join-Path -Path $dupFullPath -ChildPath $groupFolderName

    Write-MediaSiftLog "Processing Duplicate Group $groupCounter (Hash: $shortHash, Count: $($set.Count))" -LogPath $logFullPath

    if (-not $WhatIfPreference) {
        if (-not (Test-Path -Path $groupFolderPath)) {
            New-Item -ItemType Directory -Path $groupFolderPath -Force | Out-Null
        }
    }

    # First file is designated as Original
    $originalItem = $set[0].File
    $origDest = Get-DestinationPathForFile -BaseTargetFolder $origFullPath -File $originalItem -RootPath $resolvedPath -ShouldPreserveStructure $PreserveStructure.IsPresent

    if ($psCmdlet.ShouldProcess($originalItem.FullName, "Move Original copy to $origDest")) {
        Move-Item -Path $originalItem.FullName -Destination $origDest
        Write-MediaSiftLog "  [Original] Moved '$($originalItem.FullName)' -> '$origDest'" -LogPath $logFullPath
    }

    # Remaining files are moved to Duplicates group folder
    for ($i = 1; $i -lt $set.Count; $i++) {
        $dupItem = $set[$i].File
        $dupDest = Get-UniqueDestinationPath -TargetFolder $groupFolderPath -FileName $dupItem.Name

        if ($psCmdlet.ShouldProcess($dupItem.FullName, "Move Duplicate copy to $dupDest")) {
            Move-Item -Path $dupItem.FullName -Destination $dupDest
            Write-MediaSiftLog "  [Duplicate] Moved '$($dupItem.FullName)' -> '$dupDest'" -LogPath $logFullPath
        }
    }

    $groupCounter++
}

Write-MediaSiftLog "Duplicate organization completed successfully." -LogPath $logFullPath
