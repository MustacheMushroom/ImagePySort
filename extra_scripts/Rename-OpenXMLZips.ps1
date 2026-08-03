<#
.SYNOPSIS
    Inspects ZIP archives in a directory, identifies OpenXML (Word, Excel, PowerPoint, Visio, XPS)
    and OpenDocument (ODT, ODS, ODP) files, moves original ZIPs to an 'originals' folder,
    and creates renamed copies with their correct document extension.

.DESCRIPTION
    This script inspects the internal structure of .zip files (checking [Content_Types].xml,
    internal paths, and mimetype streams). If a zip file matches a known OpenXML or OpenDocument
    package, the original .zip is moved non-destructively to an 'originals' directory and
    a renamed copy with its appropriate extension (.docx, .pptx, .xlsx, .odt, .xps, etc.) is saved.

.PARAMETER Path
    The directory path to scan for .zip files. Defaults to the current working directory.

.PARAMETER OriginalsDirName
    The name of the subfolder where original ZIP files will be preserved. Defaults to 'originals'.

.PARAMETER Force
    Overwrites target files or existing archives if they already exist.

.EXAMPLE
    .\Rename-OpenXMLZips.ps1 -WhatIf
    Previews what files would be renamed without making any file changes.

.EXAMPLE
    .\Rename-OpenXMLZips.ps1
    Processes all ZIP files in the current folder.
#>

[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [Parameter(Position = 0)]
    [ValidateScript({ Test-Path $_ -PathType Container })]
    [string]$Path = '.',

    [Parameter()]
    [string]$OriginalsDirName = 'originals',

    [Parameter()]
    [switch]$Force
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.IO.Compression.FileSystem

function Get-ZipDocumentExtension {
    [CmdletBinding()]
    [OutputType([string])]
    param (
        [Parameter(Mandatory = $true)]
        [string]$ZipFilePath
    )

    $zip = $null
    try {
        $zip = [System.IO.Compression.ZipFile]::OpenRead($ZipFilePath)
        $entryNames = $zip.Entries | ForEach-Object { $_.FullName }

        # 1. Check for 'mimetype' file (OpenDocument / EPUB)
        $mimeEntry = $zip.GetEntry('mimetype')
        if ($null -ne $mimeEntry) {
            $reader = [System.IO.StreamReader]::new($mimeEntry.Open())
            $mimeType = $reader.ReadToEnd().Trim()
            $reader.Dispose()

            switch -Wildcard ($mimeType) {
                '*opendocument.text-template*'        { return '.ott' }
                '*opendocument.text*'                 { return '.odt' }
                '*opendocument.spreadsheet-template*' { return '.ots' }
                '*opendocument.spreadsheet*'          { return '.ods' }
                '*opendocument.presentation-template*' { return '.otp' }
                '*opendocument.presentation*'         { return '.odp' }
                '*opendocument.graphics-template*'     { return '.otg' }
                '*opendocument.graphics*'             { return '.odg' }
                '*opendocument.formula*'              { return '.odf' }
                '*opendocument.database*'             { return '.odb' }
                '*epub+zip*'                          { return '.epub' }
            }
        }

        # 2. Check [Content_Types].xml (Microsoft Office OpenXML / XPS / 3MF)
        $ctEntry = $zip.GetEntry('[Content_Types].xml')
        if ($null -ne $ctEntry) {
            $reader = [System.IO.StreamReader]::new($ctEntry.Open())
            $ctXml = $reader.ReadToEnd()
            $reader.Dispose()

            # WordprocessingML
            if ($ctXml -match 'officedocument\.wordprocessingml\.document') { return '.docx' }
            if ($ctXml -match 'wordprocessingml\.template') { return '.dotx' }
            if ($ctXml -match 'word\.document\.macroEnabled') { return '.docm' }
            if ($ctXml -match 'word\.template\.macroEnabled') { return '.dotm' }

            # SpreadsheetML
            if ($ctXml -match 'officedocument\.spreadsheetml\.sheet') { return '.xlsx' }
            if ($ctXml -match 'spreadsheetml\.template') { return '.xltx' }
            if ($ctXml -match 'excel\.sheet\.macroEnabled') { return '.xlsm' }
            if ($ctXml -match 'excel\.template\.macroEnabled') { return '.xltm' }
            if ($ctXml -match 'excel\.sheet\.binary') { return '.xlsb' }
            if ($ctXml -match 'excel\.addin\.macroEnabled') { return '.xlam' }

            # PresentationML
            if ($ctXml -match 'officedocument\.presentationml\.presentation') { return '.pptx' }
            if ($ctXml -match 'presentationml\.template') { return '.potx' }
            if ($ctXml -match 'presentationml\.slideshow') { return '.ppsx' }
            if ($ctXml -match 'powerpoint\.presentation\.macroEnabled') { return '.pptm' }
            if ($ctXml -match 'powerpoint\.template\.macroEnabled') { return '.potm' }
            if ($ctXml -match 'powerpoint\.slideshow\.macroEnabled') { return '.ppsm' }

            # Visio
            if ($ctXml -match 'ms-visio\.drawing') { return '.vsdx' }
            if ($ctXml -match 'ms-visio\.template') { return '.vstx' }
            if ($ctXml -match 'ms-visio\.stencil') { return '.vssx' }
            if ($ctXml -match 'ms-visio\.drawing\.macroEnabled') { return '.vsdm' }

            # XPS / 3MF
            if ($ctXml -match 'xps-fixeddocument') { return '.xps' }
            if ($ctXml -match '3dmanufacturing-3dmodel') { return '.3mf' }
        }

        # 3. Structural folder / file heuristics fallback
        if ($entryNames -like 'word/*') { return '.docx' }
        if ($entryNames -like 'xl/*') { return '.xlsx' }
        if ($entryNames -like 'ppt/*') { return '.pptx' }
        if ($entryNames -like 'visio/*') { return '.vsdx' }
        if ($entryNames -contains 'FixedDocumentSequence.fdseq') { return '.xps' }

        return $null
    }
    catch {
        Write-Warning "Could not inspect '$ZipFilePath': $_"
        return $null
    }
    finally {
        if ($null -ne $zip) {
            $zip.Dispose()
        }
    }
}

# Resolve target directory
$targetDir = (Resolve-Path $Path).Path
$originalsPath = Join-Path $targetDir $OriginalsDirName

Write-Information "Scanning directory: $targetDir" -InformationAction Continue

$zipFiles = Get-ChildItem -Path $targetDir -Filter *.zip -File
if ($zipFiles.Count -eq 0) {
    Write-Information "No .zip files found in $targetDir." -InformationAction Continue
    return
}

$renamedCount = 0
$skippedCount = 0

foreach ($file in $zipFiles) {
    $detectedExt = Get-ZipDocumentExtension -ZipFilePath $file.FullName

    if ($null -eq $detectedExt) {
        Write-Information "Skipping '$($file.Name)' (No OpenXML/ODF package structure detected)" -InformationAction Continue
        $skippedCount++
        continue
    }

    # Determine target filename
    $baseName = [System.IO.Path]::GetFileNameWithoutExtension($file.Name)
    if ($baseName.EndsWith($detectedExt, [System.StringComparison]::OrdinalIgnoreCase)) {
        $targetFileName = $baseName
    } else {
        $targetFileName = "$baseName$detectedExt"
    }

    $targetFilePath = Join-Path $targetDir $targetFileName
    $archiveFilePath = Join-Path $originalsPath $file.Name

    if ((Test-Path $targetFilePath) -and -not $Force) {
        Write-Warning "Target file '$targetFileName' already exists in '$targetDir'. Use -Force to overwrite. Skipping '$($file.Name)'."
        $skippedCount++
        continue
    }

    if ((Test-Path $archiveFilePath) -and -not $Force) {
        Write-Warning "Archive file '$archiveFilePath' already exists in '$originalsPath'. Use -Force to overwrite. Skipping '$($file.Name)'."
        $skippedCount++
        continue
    }

    Write-Information "Identified '$($file.Name)' -> '$targetFileName' ($detectedExt)" -InformationAction Continue

    if ($PSCmdlet.ShouldProcess($file.FullName, "Backup to '$archiveFilePath' and rename to '$targetFileName'")) {
        # Ensure originals directory exists
        if (-not (Test-Path $originalsPath)) {
            $null = New-Item -Path $originalsPath -ItemType Directory -Force
        }

        # Move original ZIP to originals folder
        Move-Item -Path $file.FullName -Destination $archiveFilePath -Force

        # Copy original archive back to target folder with the detected extension
        Copy-Item -Path $archiveFilePath -Destination $targetFilePath -Force
        $renamedCount++
    }
}

Write-Information "`nSummary:" -InformationAction Continue
Write-Information "  Renamed: $renamedCount file(s)" -InformationAction Continue
Write-Information "  Skipped: $skippedCount file(s)" -InformationAction Continue
Write-Information "  Originals stored in: $originalsPath" -InformationAction Continue
