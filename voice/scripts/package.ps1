<#
.SYNOPSIS
Builds Rukoo Voice's Windows installer and portable zip.

.DESCRIPTION
Builds the five release programs (rukoo-voice, rukoo-voice-recorder, rukoo-voice-local, rukoo-voice-speakers and
rukoo-voice-screen), fetches the NeMo-Speech.cpp runtime that voice/nemo-runtime/manifest.json pins, runs Tauri's
bundler for the per-user NSIS installer and checks the result. Everything goes to <cargo target dir>\package:

  Rukoo-Voice-Setup-<version>.exe     the installer
  Rukoo-Voice-Portable-<version>.zip  the five programs, the runtime folder nemo_speech\ and the notices, without an
                                      installer
  SHA256SUMS.txt                      checksums of both

It runs Tauri's CLI through npx at a pinned version, so the NSIS template matches voice/crates/voice-app/nsis.
Set SHERPA_ONNX_ARCHIVE_DIR to a folder holding the sherpa-onnx archive to skip its 123 MB download. The runtime
archive is downloaded once into <cargo target dir>\nemo-runtime\<name>. It stops before building when the runtime
isn't published yet (voice/nemo-runtime/README.md).

It checks that:
  - THIRD_PARTY_NOTICES.md lists exactly the Rust crates the programs contain (-UpdateNotices rewrites the list);
  - the installer holds the five programs, the notices and the licenses, and uses Voice's NSIS hooks and texts;
  - its nemo_speech\ folder holds exactly the runtime's files and runtime.json, with the manifest's hashes;
  - none of the five programs imports a C runtime DLL (vcruntime, msvcp or the UCRT);
  - rukoo-voice-screen.exe doesn't import Media Foundation (mfplat.dll, mfreadwrite.dll), so it starts without it;
  - the runtime's DLLs import nothing but each other, the Visual C++ runtime next to them, the UCRT and Windows, and
    that Visual C++ runtime has the versions runtime.json records;
  - rukoo-voice.exe shows the app icon first and carries the recording tray icon;
  - the hooks keep the program out of the data folder and refuse to run while a recording runs
    (voice/crates/voice-app/nsis/test-hooks.ps1, which starts a recorder on a synthetic tone and a screen process on a
    synthetic screen in the build folder).

The installer is not signed in 0.1.

.PARAMETER UpdateNotices
Rewrites the crate list in voice/THIRD_PARTY_NOTICES.md and stops, without building.

.PARAMETER RuntimeManifest
A runtime manifest to package instead of voice/nemo-runtime/manifest.json, such as the manifest.json of a runtime
workflow artifact or of pack.ps1 on this computer, to check a package before the runtime is published. Its archive URL
may be a local file (file:///E:/.../nemo-runtime-97a15af-1-win-x64.zip). The programs are built with this manifest
compiled in (RUKOO_VOICE_NEMO_MANIFEST, read by crates/voice-core/build.rs), so the packaged Voice accepts the runtime
folder it ships. Such a package is for testing only.

.EXAMPLE
pwsh voice/scripts/package.ps1
#>
[CmdletBinding()]
param(
  [switch]$UpdateNotices,
  [string]$RuntimeManifest
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

# The CLI whose NSIS template voice/crates/voice-app/nsis was written against (hooks.nsh, English.nsh, Dutch.nsh).
$TauriCli = '2.12.4'
$Triple = 'x86_64-pc-windows-msvc'
$Sidecars = @('rukoo-voice-recorder', 'rukoo-voice-local', 'rukoo-voice-speakers', 'rukoo-voice-screen')
$Programs = @('rukoo-voice') + $Sidecars
# The runtime folder next to the programs, as voice-core's speakers::RUNTIME_FOLDER names it, and the DLLs in it that
# build.ps1 compiles. The rest are the Visual C++ runtime and the licenses.
$NemoFolder = 'nemo_speech'
$NemoLibraries = @('nemo_speech_asr_c.dll', 'nemo_speech_asr.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-cpu.dll')
$NemoCrt = @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
# What those DLLs may import; the same list as voice/nemo-runtime/build.ps1.
$NemoImports = '^(nemo_speech_asr\.dll|ggml(-[a-z]+)?\.dll|msvcp140\.dll|vcruntime140(_1)?\.dll|kernel32\.dll|advapi32\.dll|dbghelp\.dll|api-ms-win-crt-[a-z0-9-]+\.dll)$'

$Voice = Split-Path -Parent $PSScriptRoot
$App = Join-Path $Voice 'crates\voice-app'
$NoticesFile = Join-Path $Voice 'THIRD_PARTY_NOTICES.md'
$Utf8 = New-Object System.Text.UTF8Encoding($false)
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
# The builds set RUKOO_VOICE_NEMO_MANIFEST (see the runtime manifest below); this puts it back on failure.
$previousNemoManifest = $env:RUKOO_VOICE_NEMO_MANIFEST
trap { $env:RUKOO_VOICE_NEMO_MANIFEST = $previousNemoManifest; break }

function Invoke-Native {
  param([string]$File, [string[]]$Arguments)
  & $File @Arguments
  if ($LASTEXITCODE -ne 0) { throw "$File $($Arguments -join ' ') failed with exit code $LASTEXITCODE" }
}

function Sort-Ordinal {
  param([string[]]$Items)
  $copy = [string[]]@($Items)
  [Array]::Sort($copy, [StringComparer]::Ordinal)
  return , $copy
}

# One spelling per license: Cargo accepts "MIT/Apache-2.0" and either order of a plain OR.
function Format-License {
  param([string]$Expression)
  $e = $Expression.Trim()
  if ($e -eq '') { return 'No license field (see the crate)' }
  if ($e -match '\bAND\b|\(') { return $e }
  $parts = @($e -split '\s*/\s*|\s+OR\s+' | Where-Object { $_ -ne '' } | Select-Object -Unique)
  return ((Sort-Ordinal $parts) -join ' OR ')
}

# The crates linked into the five programs: normal dependencies on Windows x64, without procedural macros (they run
# only while compiling) and without Voice's own crates, which have a path instead of a registry source.
function Get-ShippedCrates {
  Push-Location $Voice
  try {
    $lines = & cargo tree --locked -e 'normal,no-proc-macro' --target $Triple -p voice-app -p voice-recorder -p voice-local -p voice-speakers -p voice-screen --prefix none -f '{p}|{l}'
    if ($LASTEXITCODE -ne 0) { throw 'cargo tree failed' }
  } finally { Pop-Location }
  $seen = @{}
  $crates = @()
  foreach ($line in $lines) {
    $line = ($line -replace ' \(\*\)$', '').Trim()
    if ($line -notmatch '^(?<name>\S+) v(?<version>\S+)(?<source> \(.+\))?\|(?<license>.*)$') { continue }
    if ($Matches['source']) { continue }
    $key = "$($Matches['name']) $($Matches['version'])"
    if ($seen.ContainsKey($key)) { continue }
    $seen[$key] = $true
    $crates += New-Object PSObject -Property @{
      Name = $Matches['name']; Version = $Matches['version']; License = (Format-License $Matches['license'])
    }
  }
  if ($crates.Count -eq 0) { throw 'cargo tree listed no crates' }
  return $crates
}

function Get-CrateBlock {
  param($Crates)
  $out = New-Object System.Text.StringBuilder
  [void]$out.Append("`n$($Crates.Count) crates.`n")
  $licenses = Sort-Ordinal @($Crates | ForEach-Object { $_.License } | Select-Object -Unique)
  foreach ($license in $licenses) {
    $group = @($Crates | Where-Object { $_.License -ceq $license })
    [void]$out.Append("`n### $license`n`n")
    foreach ($entry in (Sort-Ordinal @($group | ForEach-Object { "$($_.Name) $($_.Version)" }))) {
      [void]$out.Append("- $entry`n")
    }
  }
  return $out.ToString() + "`n"
}

# Rewrites or checks the generated part of THIRD_PARTY_NOTICES.md, between its two marker comments.
function Sync-Notices {
  param($Crates, [switch]$Write)
  $text = [IO.File]::ReadAllText($NoticesFile)
  $newline = "`n"
  if ($text.Contains("`r`n")) { $newline = "`r`n" }
  $text = $text.Replace("`r`n", "`n")
  $pattern = '(?s)(<!-- rust-crates:start[^\n]*-->\n).*?(<!-- rust-crates:end -->)'
  $match = [regex]::Match($text, $pattern)
  if (-not $match.Success) { throw "$NoticesFile has no rust-crates markers" }
  $block = Get-CrateBlock $Crates
  $updated = $text.Substring(0, $match.Index) + $match.Groups[1].Value + $block + $match.Groups[2].Value +
    $text.Substring($match.Index + $match.Length)
  if ($updated -ceq $text) { return $true }
  if ($Write) {
    [IO.File]::WriteAllText($NoticesFile, $updated.Replace("`n", $newline), $Utf8)
    return $true
  }
  return $false
}

# THIRD_PARTY_LICENSES.txt: the license files each crate's source package carries, plus Inter's and NeMo-Speech.cpp's
# from the runtime folder.
function Write-LicenseTexts {
  param($Crates, [string]$Path, [string]$NemoDir)
  Push-Location $Voice
  try {
    $json = & cargo metadata --locked --format-version 1 --filter-platform $Triple
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
  } finally { Pop-Location }
  $packages = @{}
  foreach ($p in (($json -join "`n") | ConvertFrom-Json).packages) { $packages["$($p.name) $($p.version)"] = $p }
  $out = New-Object System.Text.StringBuilder
  [void]$out.Append("License files of the third-party code in Rukoo Voice. THIRD_PARTY_NOTICES.md summarizes them.`n")
  $inter = Join-Path $Voice 'ui\fonts\Inter-LICENSE.txt'
  [void]$out.Append("`n==== Inter (voice/ui/fonts) ====`n`n" + [IO.File]::ReadAllText($inter).Trim() + "`n")
  # NeMo-Speech.cpp's LICENSE, NOTICE and THIRD_PARTY_NOTICES.md, then the licenses of what vcpkg linked into it.
  [void]$out.Append("`n==== NeMo-Speech.cpp and the libraries in its DLLs ($NemoFolder\licenses) ====`n")
  $licenseDir = Join-Path $NemoDir 'licenses'
  foreach ($file in (Get-ChildItem -LiteralPath $licenseDir -Recurse -File | Sort-Object { $_.FullName.Substring($licenseDir.Length) })) {
    $name = $file.FullName.Substring($licenseDir.Length + 1).Replace('\', '/')
    [void]$out.Append("`n-- $name`n`n" + [IO.File]::ReadAllText($file.FullName).Replace("`r`n", "`n").Trim() + "`n")
  }
  foreach ($key in (Sort-Ordinal @($Crates | ForEach-Object { "$($_.Name) $($_.Version)" }))) {
    $crate = $Crates | Where-Object { "$($_.Name) $($_.Version)" -ceq $key } | Select-Object -First 1
    $package = $packages[$key]
    if (-not $package) { throw "cargo metadata has no package $key" }
    $dir = Split-Path -Parent $package.manifest_path
    $files = @(Get-ChildItem -LiteralPath $dir -File | Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|UNLICENSE)' })
    if ($package.license_file) {
      $named = Join-Path $dir $package.license_file
      if ((Test-Path -LiteralPath $named) -and -not ($files | Where-Object { $_.FullName -eq (Resolve-Path -LiteralPath $named).Path })) {
        $files += Get-Item -LiteralPath $named
      }
    }
    [void]$out.Append("`n==== $key ($($crate.License)) ====`n")
    if ($files.Count -eq 0) {
      [void]$out.Append("`nThe package has no license file. Its license is $($crate.License).`n")
    }
    foreach ($file in ($files | Sort-Object Name)) {
      [void]$out.Append("`n-- $($file.Name)`n`n" + [IO.File]::ReadAllText($file.FullName).Replace("`r`n", "`n").Trim() + "`n")
    }
  }
  [IO.File]::WriteAllText($Path, $out.ToString().Replace("`n", "`r`n"), $Utf8)
}

function Get-Dumpbin {
  $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
  if (-not (Test-Path -LiteralPath $vswhere)) { throw 'vswhere.exe not found; install the Visual Studio C++ build tools' }
  $dumpbin = & $vswhere -latest -products * -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe' | Select-Object -First 1
  if (-not $dumpbin) { throw 'dumpbin.exe not found; install the Visual Studio C++ build tools' }
  return $dumpbin
}

# The DLLs a program or library imports, as dumpbin lists them.
function Get-Imports {
  param([string]$Dumpbin, [string]$File)
  $imports = @(& $Dumpbin /nologo /dependents $File | Where-Object { $_ -match '^\s+\S+\.dll\s*$' } | ForEach-Object { $_.Trim() })
  if ($LASTEXITCODE -ne 0 -or $imports.Count -eq 0) { throw "dumpbin could not read $File" }
  return $imports
}

# Fails when a program imports a C runtime DLL, so it would need the Visual C++ redistributable or the UCRT forwarders.
function Assert-StaticCrt {
  param([string[]]$Files)
  $dumpbin = Get-Dumpbin
  foreach ($file in $Files) {
    $imports = @(Get-Imports $dumpbin $file)
    $crt = @($imports | Where-Object { $_ -match '^(vcruntime|msvcp|ucrtbase|api-ms-win-crt-)' })
    if ($crt.Count -gt 0) { throw "$(Split-Path -Leaf $file) imports the C runtime: $($crt -join ', ')" }
    Write-Host "  $(Split-Path -Leaf $file): static C runtime ($($imports.Count) DLL imports, none from the CRT)"
  }
}

# Fails when the screen process imports Media Foundation. It loads mfplat.dll and mfreadwrite.dll itself when it
# records video, so on a Windows N edition without them it still starts and records frames (voice/RECORDING.md).
function Assert-NoLoadTimeMediaFoundation {
  param([string]$File)
  $imports = @(Get-Imports (Get-Dumpbin) $File)
  $mf = @($imports | Where-Object { $_ -match '^(mfplat|mfreadwrite)\.dll$' })
  if ($mf.Count -gt 0) { throw "$(Split-Path -Leaf $File) imports Media Foundation: $($mf -join ', ')" }
  Write-Host "  $(Split-Path -Leaf $File): no Media Foundation import ($($imports.Count) DLL imports)"
}

# The NeMo DLLs use the dynamic C runtime, which ships next to them. Fails when one of them needs anything outside the
# folder, the UCRT and Windows, uses OpenMP, or when the Visual C++ runtime files aren't the versions the build
# recorded.
function Assert-NemoRuntime {
  param([string]$Dir)
  $dumpbin = Get-Dumpbin
  foreach ($name in $NemoLibraries) {
    $imports = @(Get-Imports $dumpbin (Join-Path $Dir $name))
    if ($imports | Where-Object { $_ -match '^vcomp' }) { throw "$NemoFolder\$name uses OpenMP: $($imports -join ', ')" }
    $unexpected = @($imports | Where-Object { $_ -notmatch $NemoImports })
    if ($unexpected.Count -gt 0) { throw "$NemoFolder\$name imports $($unexpected -join ', '), which the folder doesn't ship" }
    Write-Host "  $NemoFolder\${name}: imports $($imports -join ', ')"
  }
  $runtime = Get-Content -LiteralPath (Join-Path $Dir 'runtime.json') -Raw | ConvertFrom-Json
  foreach ($name in $NemoCrt) {
    $entry = $runtime.files | Where-Object { $_.path -eq $name } | Select-Object -First 1
    if (-not $entry -or -not $entry.PSObject.Properties['version']) { throw "runtime.json records no version for $name" }
    $info = (Get-Item -LiteralPath (Join-Path $Dir $name)).VersionInfo
    $version = "$($info.FileMajorPart).$($info.FileMinorPart).$($info.FileBuildPart).$($info.FilePrivatePart)"
    if ($version -ne $entry.version) { throw "$NemoFolder\$name is version $version, but runtime.json says $($entry.version)" }
    Write-Host "  $NemoFolder\${name}: Visual C++ runtime $version"
  }
}

# The runtime manifest to package: voice/nemo-runtime/manifest.json, or -RuntimeManifest. Stops when it names no
# archive, which means the runtime isn't published yet.
function Read-RuntimeManifest {
  param([string]$Path)
  $manifest = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json
  # Format 2 holds one entry per platform. This package ships the Windows runtime, and the rest of
  # the script reads name, commit, archive and files from the top of the object.
  if ($manifest.format -eq 2) {
    $target = $manifest.targets.'windows-x86_64'
    if (-not $target) { throw "The runtime manifest $Path pins no windows-x86_64 runtime" }
    foreach ($property in $target.PSObject.Properties) {
      $manifest = $manifest | Add-Member -NotePropertyName $property.Name -NotePropertyValue $property.Value -Force -PassThru
    }
  }
  if (-not $manifest.archive -or @($manifest.files).Count -eq 0) {
    throw 'Publish the NeMo runtime first (voice/nemo-runtime/README.md). voice/nemo-runtime/manifest.json has no archive yet.'
  }
  return $manifest
}

# Problems with a runtime folder: a manifest file missing or different, runtime.json missing or naming another build,
# or a file the manifest doesn't list. Returns $null when it matches.
function Test-NemoFolder {
  param($Manifest, [string]$Dir)
  $runtimeJson = Join-Path $Dir 'runtime.json'
  if (-not (Test-Path -LiteralPath $runtimeJson)) { return 'runtime.json is missing' }
  $runtime = Get-Content -LiteralPath $runtimeJson -Raw | ConvertFrom-Json
  if ($runtime.name -ne $Manifest.name -or $runtime.commit -ne $Manifest.commit) { return "runtime.json describes $($runtime.name)" }
  $expected = @('runtime.json')
  foreach ($file in $Manifest.files) {
    $expected += $file.path
    $full = Join-Path $Dir $file.path
    if (-not (Test-Path -LiteralPath $full -PathType Leaf)) { return "$($file.path) is missing" }
    if ((Get-Item -LiteralPath $full).Length -ne $file.size -or (Get-Sha256 $full) -ne $file.sha256) { return "$($file.path) differs from the manifest" }
  }
  $present = @(Get-ChildItem -LiteralPath $Dir -Recurse -File | ForEach-Object { $_.FullName.Substring($Dir.Length + 1).Replace('\', '/') })
  $extra = @($present | Where-Object { $expected -notcontains $_ })
  if ($extra.Count -gt 0) { return "it also holds $($extra -join ', ')" }
  return $null
}

# Downloads the runtime archive once into <target>\nemo-runtime\<name>, checks it, and unpacks a checked copy of the
# runtime folder to $Destination.
function Get-NemoRuntime {
  param($Manifest, [string]$Target, [string]$Destination)
  $cache = Join-Path $Target "nemo-runtime\$($Manifest.name)"
  New-Item -ItemType Directory -Force -Path $cache | Out-Null
  $source = [uri]$Manifest.archive.url
  $zip = Join-Path $cache (Split-Path -Leaf $source.LocalPath)
  $fresh = $true
  if (Test-Path -LiteralPath $zip) {
    $fresh = ((Get-Item -LiteralPath $zip).Length -ne $Manifest.archive.size) -or ((Get-Sha256 $zip) -ne $Manifest.archive.sha256)
  }
  if ($fresh) {
    $part = "$zip.part"
    if ($source.IsFile) {
      Copy-Item -LiteralPath $source.LocalPath -Destination $part -Force
    } else {
      Write-Host "  downloading $($source.AbsoluteUri)"
      $progress = $ProgressPreference
      $ProgressPreference = 'SilentlyContinue'
      [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
      try { Invoke-WebRequest -UseBasicParsing -Uri $source.AbsoluteUri -OutFile $part } finally { $ProgressPreference = $progress }
    }
    $size = (Get-Item -LiteralPath $part).Length
    $hash = Get-Sha256 $part
    if ($size -ne $Manifest.archive.size -or $hash -ne $Manifest.archive.sha256) {
      Remove-Item -LiteralPath $part -Force
      throw "The runtime archive $($source.AbsoluteUri) has $size bytes and SHA-256 $hash instead of $($Manifest.archive.size) and $($Manifest.archive.sha256)"
    }
    Move-Item -LiteralPath $part -Destination $zip -Force
  }
  if (Test-Path -LiteralPath $Destination) { Remove-Item -LiteralPath $Destination -Recurse -Force }
  [IO.Compression.ZipFile]::ExtractToDirectory($zip, $Destination)
  $problem = Test-NemoFolder $Manifest $Destination
  if ($problem) { throw "The runtime archive doesn't match the manifest: $problem" }
  Write-Host "  $($Manifest.name): $(@($Manifest.files).Count) files and runtime.json checked"
}

# A zip of a folder's files with forward slashes in the entry names; Compress-Archive in Windows PowerShell writes
# backslashes.
function Write-Zip {
  param([string]$Dir, [string]$Path)
  if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Force }
  $zip = [IO.Compression.ZipFile]::Open($Path, [IO.Compression.ZipArchiveMode]::Create)
  try {
    foreach ($file in (Get-ChildItem -LiteralPath $Dir -Recurse -File | Sort-Object FullName)) {
      $name = $file.FullName.Substring($Dir.Length + 1).Replace('\', '/')
      [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $file.FullName, $name, [IO.Compression.CompressionLevel]::Optimal)
    }
  } finally { $zip.Dispose() }
}

# The exe's icon groups in resource order. Explorer, shortcuts and Apps & features show the first one, so that must be
# the app icon (32512, from tauri-build); build.rs adds the recording tray icon as 32513.
function Assert-IconResources {
  param([string]$Exe)
  if (-not ('VoicePackage.Icons' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
namespace VoicePackage {
  public static class Icons {
    delegate bool EnumNames(IntPtr module, IntPtr type, IntPtr name, IntPtr param);
    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    static extern IntPtr LoadLibraryExW(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll")] static extern bool FreeLibrary(IntPtr module);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode)]
    static extern bool EnumResourceNamesW(IntPtr module, IntPtr type, EnumNames callback, IntPtr param);
    public static string[] Groups(string path) {
      // LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE: reads the resources, runs nothing.
      IntPtr module = LoadLibraryExW(path, IntPtr.Zero, 0x22);
      if (module == IntPtr.Zero) throw new System.ComponentModel.Win32Exception();
      var names = new List<string>();
      try {
        EnumResourceNamesW(module, (IntPtr)14, (m, t, n, p) => {
          long id = n.ToInt64();
          names.Add((id >> 16) == 0 ? id.ToString() : Marshal.PtrToStringUni(n));
          return true;
        }, IntPtr.Zero);
      } finally { FreeLibrary(module); }
      return names.ToArray();
    }
  }
}
'@
  }
  $groups = @([VoicePackage.Icons]::Groups($Exe))
  if ($groups.Count -eq 0 -or $groups[0] -ne '32512') { throw "The first icon in $Exe is $($groups -join ', ') instead of the app icon 32512" }
  if ($groups -notcontains '32513') { throw "$Exe has no recording tray icon (32513)" }
  Write-Host "  $(Split-Path -Leaf $Exe): icons $($groups -join ', '), the app icon first"
}

# Unpacks the NSIS installer with 7-Zip, which reads the archive without running the installer.
function Expand-Installer {
  param([string]$Installer, [string]$Destination)
  $path = $null
  $command = Get-Command 7z -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($command) { $path = $command.Path }
  if (-not $path) {
    $default = Join-Path $env:ProgramFiles '7-Zip\7z.exe'
    if (Test-Path -LiteralPath $default) { $path = $default }
  }
  if (-not $path) { throw '7-Zip (7z) is needed to check the installer' }
  & $path x -y "-o$Destination" $Installer | Out-Null
  if ($LASTEXITCODE -ne 0) { throw "7-Zip could not unpack $Installer" }
}

function Get-Sha256 {
  param([string]$Path)
  return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$crates = Get-ShippedCrates
if ($UpdateNotices) {
  [void](Sync-Notices $crates -Write)
  Write-Host "Updated the crate list in $NoticesFile ($($crates.Count) crates)."
  return
}
if (-not (Sync-Notices $crates)) {
  throw "The crate list in $NoticesFile is out of date. Run voice/scripts/package.ps1 -UpdateNotices and commit the change."
}
$testRuntime = [bool]$RuntimeManifest
if (-not $RuntimeManifest) { $RuntimeManifest = Join-Path $Voice 'nemo-runtime\manifest.json' }
$RuntimeManifest = (Resolve-Path -LiteralPath $RuntimeManifest).Path
$nemoManifest = Read-RuntimeManifest $RuntimeManifest
# voice-core compiles in the manifest it checks nemo_speech\ against (crates/voice-core/build.rs). The builds below get
# the one packaged here, so a test package of an unpublished runtime also accepts the folder it ships. The variable is
# put back when the script ends, also on failure (the trap at the top).
$env:RUKOO_VOICE_NEMO_MANIFEST = $RuntimeManifest
if ($testRuntime) {
  Write-Warning "Packaging the runtime of $RuntimeManifest instead of voice/nemo-runtime/manifest.json: this package is for testing only."
}

Push-Location $Voice
try {
  $meta = (& cargo metadata --locked --no-deps --format-version 1) -join "`n" | ConvertFrom-Json
  if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
} finally { Pop-Location }
$version = ($meta.packages | Where-Object { $_.name -eq 'voice-app' }).version
$target = $meta.target_directory
# Tauri's bundler drops the drive letter of sidecar and resource paths on another drive than the app's crate, and then
# can't find them, after the whole release build.
$targetDrive = (Split-Path -Qualifier $target).ToUpperInvariant()
$appDrive = (Split-Path -Qualifier $App).ToUpperInvariant()
if ($targetDrive -ne $appDrive) {
  throw "The cargo target folder $target is on $targetDrive and the checkout on ${appDrive}: set CARGO_TARGET_DIR to a folder on $appDrive for packaging."
}
$release = Join-Path $target 'release'
$out = Join-Path $target 'package'
Write-Host "Rukoo Voice $version, building in $target"

if (Test-Path -LiteralPath $out) { Remove-Item -LiteralPath $out -Recurse -Force }
$staging = Join-Path $out 'sidecars'
New-Item -ItemType Directory -Force -Path $staging | Out-Null

Write-Host "Fetching the NeMo runtime $($nemoManifest.name)"
$nemo = Join-Path $out $NemoFolder
Get-NemoRuntime $nemoManifest $target $nemo

Write-Host 'Building the recorder, the local speech worker, the speaker worker and the screen process'
Push-Location $Voice
try {
  Invoke-Native cargo @('build', '--release', '--locked', '-p', 'voice-recorder', '-p', 'voice-local', '-p', 'voice-speakers', '-p', 'voice-screen')
} finally { Pop-Location }

# Tauri's externalBin wants each program named with the target triple. The installer puts them next to rukoo-voice.exe
# under their plain names, where the engine looks for them.
$externalBin = @()
foreach ($name in $Sidecars) {
  Copy-Item -LiteralPath (Join-Path $release "$name.exe") -Destination (Join-Path $staging "$name-$Triple.exe")
  $externalBin += (Join-Path $staging $name).Replace('\', '/')
}
$licenses = Join-Path $out 'THIRD_PARTY_LICENSES.txt'
Write-LicenseTexts $crates $licenses $nemo
# Merged over tauri.conf.json by the CLI only, so a plain `cargo build -p voice-app` never needs the sidecars.
$resources = @{}
$resources[$licenses.Replace('\', '/')] = 'THIRD_PARTY_LICENSES.txt'
foreach ($path in @('runtime.json') + @($nemoManifest.files | ForEach-Object { $_.path })) {
  $resources[(Join-Path $nemo $path).Replace('\', '/')] = "$NemoFolder/$path"
}
$patch = @{ bundle = @{ externalBin = $externalBin; resources = $resources } }
$patchFile = Join-Path $out 'tauri.package.json'
[IO.File]::WriteAllText($patchFile, ($patch | ConvertTo-Json -Depth 5), $Utf8)

Write-Host "Building the app and the installer with @tauri-apps/cli $TauriCli"
Push-Location $App
try {
  Invoke-Native 'npx.cmd' @('--yes', "@tauri-apps/cli@$TauriCli", 'build', '--bundles', 'nsis', '--config', $patchFile, '--ci')
} finally { Pop-Location }

$built = Join-Path $release "bundle\nsis\Rukoo Voice_${version}_x64-setup.exe"
if (-not (Test-Path -LiteralPath $built)) { throw "The bundler made no installer at $built" }
$installer = Join-Path $out "Rukoo-Voice-Setup-$version.exe"
Copy-Item -LiteralPath $built -Destination $installer

Write-Host 'Checking the installer'
# The script Tauri generated must include Voice's hooks, and its language files must be Voice's copies.
$scriptDir = Join-Path $release 'nsis\x64'
$nsi = [IO.File]::ReadAllText((Join-Path $scriptDir 'installer.nsi'))
if (-not $nsi.Contains('!include "' + (Join-Path $App 'nsis\hooks.nsh') + '"')) { throw 'The installer script does not include nsis\hooks.nsh' }
foreach ($language in @('English', 'Dutch')) {
  if (-not ([IO.File]::ReadAllText((Join-Path $scriptDir "$language.nsh"))).Contains('voiceKeepsData')) {
    throw "The installer's $language texts are not Voice's nsis\$language.nsh"
  }
}
# The files the installer puts in the install folder, next to each other. NSIS's own plugins unpack to $PLUGINSDIR.
# 7-Zip 26 also unpacks uninstall.exe, which the installer writes when it runs; 7-Zip 25 leaves it out.
$contents = Join-Path $out 'installer-contents'
Expand-Installer $installer $contents
$installed = @(Get-ChildItem -LiteralPath $contents -File | Where-Object { $_.Name -ne 'uninstall.exe' } | ForEach-Object { $_.Name } | Sort-Object)
$expected = @('rukoo-voice.exe', 'rukoo-voice-local.exe', 'rukoo-voice-recorder.exe', 'rukoo-voice-screen.exe', 'rukoo-voice-speakers.exe',
  'THIRD_PARTY_LICENSES.txt', 'THIRD_PARTY_NOTICES.md') | Sort-Object
if (($installed -join ',') -ne ($expected -join ',')) { throw "The installer holds $($installed -join ', ') instead of $($expected -join ', ')" }
# The sidecars and the notices are the files built here. The bundler marks its own copy of rukoo-voice.exe with the
# bundle type, so that one only gets the C runtime check.
$same = @{ 'rukoo-voice-recorder.exe' = (Join-Path $release 'rukoo-voice-recorder.exe'); 'rukoo-voice-local.exe' = (Join-Path $release 'rukoo-voice-local.exe');
  'rukoo-voice-speakers.exe' = (Join-Path $release 'rukoo-voice-speakers.exe'); 'rukoo-voice-screen.exe' = (Join-Path $release 'rukoo-voice-screen.exe');
  'THIRD_PARTY_NOTICES.md' = $NoticesFile; 'THIRD_PARTY_LICENSES.txt' = $licenses }
foreach ($name in $same.Keys) {
  if ((Get-Sha256 (Join-Path $contents $name)) -ne (Get-Sha256 $same[$name])) { throw "The installer's $name differs from $($same[$name])" }
}
# The runtime folder holds exactly the published files and runtime.json, as checked after the download.
$installedNemo = Join-Path $contents $NemoFolder
$problem = Test-NemoFolder $nemoManifest $installedNemo
if ($problem) { throw "The installer's $NemoFolder folder doesn't match the runtime manifest: $problem" }
if ((Get-Sha256 (Join-Path $installedNemo 'runtime.json')) -ne (Get-Sha256 (Join-Path $nemo 'runtime.json'))) { throw "The installer's $NemoFolder\runtime.json differs from the runtime's" }
Get-ChildItem -LiteralPath $contents -File | Sort-Object Name | ForEach-Object { Write-Host ("  {0,12:N0}  {1}" -f $_.Length, $_.Name) }
Get-ChildItem -LiteralPath $installedNemo -Recurse -File | Sort-Object FullName | ForEach-Object {
  Write-Host ("  {0,12:N0}  {1}" -f $_.Length, $_.FullName.Substring($contents.Length + 1))
}

Write-Host 'Checking the C runtime, Media Foundation, the NeMo runtime and the icons'
Assert-StaticCrt @(@($Programs | ForEach-Object { Join-Path $release "$_.exe" }) + (Join-Path $contents 'rukoo-voice.exe'))
Assert-NoLoadTimeMediaFoundation (Join-Path $release 'rukoo-voice-screen.exe')
Assert-NemoRuntime $installedNemo
Assert-IconResources (Join-Path $contents 'rukoo-voice.exe')

Write-Host 'Testing the installer hooks'
& (Join-Path $App 'nsis\test-hooks.ps1') -Recorder (Join-Path $release 'rukoo-voice-recorder.exe') -Screen (Join-Path $release 'rukoo-voice-screen.exe') `
  -Work (Join-Path $out 'hooks-test')

Write-Host 'Writing the portable zip'
$portable = Join-Path $out 'portable'
New-Item -ItemType Directory -Force -Path $portable | Out-Null
foreach ($name in $Programs) { Copy-Item -LiteralPath (Join-Path $release "$name.exe") -Destination $portable }
Copy-Item -LiteralPath $NoticesFile -Destination $portable
Copy-Item -LiteralPath $licenses -Destination $portable
Copy-Item -LiteralPath $nemo -Destination (Join-Path $portable $NemoFolder) -Recurse
$zip = Join-Path $out "Rukoo-Voice-Portable-$version.zip"
Write-Zip $portable $zip

$sums = foreach ($file in @($installer, $zip)) {
  "$((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant())  $(Split-Path -Leaf $file)"
}
[IO.File]::WriteAllText((Join-Path $out 'SHA256SUMS.txt'), (($sums -join "`n") + "`n"), $Utf8)
$env:RUKOO_VOICE_NEMO_MANIFEST = $previousNemoManifest

Write-Host ''
foreach ($file in @($installer, $zip)) {
  Write-Host ("{0}  {1:N1} MB" -f $file, ((Get-Item -LiteralPath $file).Length / 1MB))
}
