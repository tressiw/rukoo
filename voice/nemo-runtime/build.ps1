<#
.SYNOPSIS
Builds the NeMo-Speech.cpp runtime that Rukoo Voice's speaker worker loads.

.DESCRIPTION
Builds NVIDIA NeMo-Speech.cpp at the commit pinned in manifest.json (next to this script) with the CPU backend and
the asr profile, which contains diarization, and stages the runtime folder Voice ships as nemo_speech\:

  nemo_speech_asr_c.dll, nemo_speech_asr.dll, ggml.dll, ggml-base.dll, ggml-cpu.dll
  msvcp140.dll, vcruntime140.dll, vcruntime140_1.dll    the Visual C++ runtime the DLLs need, from this toolset
  licenses\LICENSE, NOTICE, THIRD_PARTY_NOTICES.md      NeMo-Speech.cpp's own
  licenses\darts-clone.txt                              the license of Darts-clone, which SentencePiece compiles in,
                                                        from licenses\ next to this script
  licenses\vcpkg\<package>.txt                          the licenses of what vcpkg linked in: SentencePiece and
                                                        the libraries it uses
  runtime.json                                          manifest.json without the archive: pins, flags, toolset,
                                                        and the size and SHA-256 of every file

The build is portable: an AVX2 baseline with FMA, F16C and BMI2, nothing past it, and no OpenMP. The flags are
seeded into CMakeCache.txt before the first configure, because NeMo's own driver passes no ggml flags, and read back
afterwards. Every DLL's imports are checked, so a build that would need anything outside the folder fails here.

It runs NeMo's scripts\windows\build.ps1, which needs Git, Visual Studio 2022 with the C++ workload, CMake 3.26 or
later and Ninja. CMake and Ninja are installed into a Python virtual environment in the build folder when they are
missing. NeMo's driver bootstraps vcpkg under %LOCALAPPDATA%\NeMoSpeech and builds SentencePiece with it; vcpkg keeps
its binary cache in %LOCALAPPDATA%\vcpkg\archives.

voice/nemo-runtime/README.md says how a build becomes the published runtime.

.PARAMETER Out
The runtime folder to write. It is replaced.

.PARAMETER Source
A NeMo-Speech.cpp checkout. Cloned when missing, then checked out at the pinned commit. Defaults to a NeMo-Speech.cpp
folder next to -Out.

.PARAMETER BuildDir
CMake's build folder. Defaults to build-voice-cpu in -Source. A second run reuses it.

.PARAMETER Jobs
Parallel compile jobs. Defaults to the number of logical processors.

.PARAMETER Runner
Where the build ran, recorded in runtime.json: the workflow passes its runner image.

.EXAMPLE
powershell -File voice\nemo-runtime\build.ps1 -Out tmp\nemo\nemo_speech -Jobs 6
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Out,
  [string]$Source,
  [string]$BuildDir,
  [int]$Jobs = [Environment]::ProcessorCount,
  [string]$Runner = 'local'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$Manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'manifest.json') -Raw | ConvertFrom-Json
# manifest.json is format 2: one entry per platform. This build is the Windows one, and the rest of
# the script reads its build, flags, vcpkg and cpu from there.
$Target = $Manifest.targets.'windows-x86_64'
if (-not $Target) { throw 'voice/nemo-runtime/manifest.json pins no windows-x86_64 runtime' }
$Libraries = @('nemo_speech_asr_c.dll', 'nemo_speech_asr.dll', 'ggml.dll', 'ggml-base.dll', 'ggml-cpu.dll')
$Crt = @('msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll')
$Licenses = @('LICENSE', 'NOTICE', 'THIRD_PARTY_NOTICES.md')
# SentencePiece compiles in Darts-clone (its third_party\darts_clone), whose BSD license wants its notice with every
# binary. vcpkg's port installs only SentencePiece's own LICENSE, and on a binary-cache hit the source isn't there, so
# the text is kept next to this script: Darts-clone's LICENSE from SentencePiece 0.2.1, unchanged.
$OwnLicenses = @('darts-clone.txt')
$Utf8 = New-Object System.Text.UTF8Encoding($false)

function Invoke-Native {
  param([string]$File, [string[]]$Arguments)
  & $File @Arguments
  if ($LASTEXITCODE -ne 0) { throw "$File $($Arguments -join ' ') failed with exit code $LASTEXITCODE" }
}

# Windows PowerShell turns a native command's redirected stderr into an error record, which stops the script.
function Invoke-Probe {
  param([string]$File, [string[]]$Arguments)
  $previous = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try {
    $output = & $File @Arguments 2>$null
    return New-Object PSObject -Property @{ ExitCode = $LASTEXITCODE; Output = @($output) }
  } finally { $ErrorActionPreference = $previous }
}

function Test-Tool {
  param([string]$Name, [version]$Minimum)
  $command = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $command) { return $false }
  if (-not $Minimum) { return $true }
  $first = (Invoke-Probe $command.Source @('--version')).Output | Select-Object -First 1
  return ($first -match '(\d+\.\d+(\.\d+)?)') -and ([version]$Matches[1] -ge $Minimum)
}

function Get-Sha256 {
  param([string]$Path)
  return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-FileVersionText {
  param([string]$Path)
  $info = (Get-Item -LiteralPath $Path).VersionInfo
  return "$($info.FileMajorPart).$($info.FileMinorPart).$($info.FileBuildPart).$($info.FilePrivatePart)"
}

function Get-VisualStudio {
  $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
  if (-not (Test-Path -LiteralPath $vswhere)) { throw 'vswhere.exe not found; install the Visual Studio 2022 C++ build tools' }
  # The same query NeMo's driver uses to pick the toolset, so the runtime DLLs come from the install that compiled.
  $path = & $vswhere -latest -products * -requires 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64' -property installationPath
  if (-not $path) { throw 'No Visual Studio install with the x64 C++ tools found' }
  return $path
}

function Resolve-Full {
  param([string]$Path)
  if (-not [IO.Path]::IsPathRooted($Path)) { $Path = Join-Path (Get-Location).Path $Path }
  return [IO.Path]::GetFullPath($Path)
}

$Out = Resolve-Full $Out
if (-not $Source) { $Source = Join-Path (Split-Path -Parent $Out) 'NeMo-Speech.cpp' }
$Source = Resolve-Full $Source
if (-not $BuildDir) { $BuildDir = Join-Path $Source 'build-voice-cpu' }
$BuildDir = Resolve-Full $BuildDir
$commit = $Manifest.commit
$short = $commit.Substring(0, 7)

Write-Host "[nemo-runtime] $($Manifest.name): NeMo-Speech.cpp $short in $Source"
if (-not (Test-Path -LiteralPath (Join-Path $Source '.git'))) {
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Source) | Out-Null
  Invoke-Native git @('clone', '--quiet', '--filter=blob:none', '--no-checkout', $Manifest.repository, $Source)
}
if ((Invoke-Probe git @('-C', $Source, 'rev-parse', '--verify', '--quiet', "$commit^{commit}")).ExitCode -ne 0) {
  Invoke-Native git @('-C', $Source, 'fetch', '--quiet', 'origin', $commit)
}
Invoke-Native git @('-C', $Source, 'checkout', '--quiet', '--detach', $commit)
# ASR builds read miniaudio from llama.cpp, so a shallow copy of both submodules is enough.
Invoke-Native git @('-C', $Source, 'submodule', 'update', '--init', '--depth', '1', 'ggml', 'llama.cpp')
foreach ($name in @('ggml', 'llama.cpp')) {
  $head = (Invoke-Probe git @('-C', (Join-Path $Source $name), 'rev-parse', 'HEAD')).Output | Select-Object -First 1
  $pinned = $Manifest.submodules.$name
  if ($head -ne $pinned) { throw "Submodule $name is at $head instead of $pinned" }
}

if (-not (Test-Tool 'cmake' ([version]'3.26')) -or -not (Test-Tool 'ninja')) {
  $tools = Join-Path $BuildDir '..\voice-tools'
  $tools = [IO.Path]::GetFullPath($tools)
  if (-not (Test-Path -LiteralPath (Join-Path $tools 'Scripts\cmake.exe'))) {
    $python = Get-Command 'py.exe' -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $python) { throw 'CMake 3.26+ and Ninja are missing, and py.exe is not there to install them' }
    Write-Host "[nemo-runtime] Installing CMake and Ninja into $tools"
    Invoke-Native $python.Source @('-m', 'venv', $tools)
    Invoke-Native (Join-Path $tools 'Scripts\python.exe') @('-m', 'pip', 'install', '--disable-pip-version-check', '-q', 'cmake', 'ninja')
  }
  $env:Path = (Join-Path $tools 'Scripts') + ';' + $env:Path
}

# NeMo's driver passes no ggml flags, so the portable set goes into the cache before the first configure.
$cachePath = Join-Path $BuildDir 'CMakeCache.txt'
if (-not (Test-Path -LiteralPath $cachePath)) {
  New-Item -ItemType Directory -Force -Path $BuildDir | Out-Null
  $seed = foreach ($flag in $Target.flags.PSObject.Properties) { "$($flag.Name):BOOL=$($flag.Value)" }
  [IO.File]::WriteAllText($cachePath, (($seed -join "`n") + "`n"), [Text.Encoding]::ASCII)
}

Write-Host '[nemo-runtime] Building the CPU runtime (asr profile, which contains diarization)'
# NeMo's driver targets Windows PowerShell, also when this script runs under pwsh.
$windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
Invoke-Native $windowsPowerShell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', (Join-Path $Source 'scripts\windows\build.ps1'),
  '-Backend', 'cpu', '-Profile', 'asr', '-BuildDir', $BuildDir, '-Jobs', "$Jobs")

$cache = Get-Content -LiteralPath $cachePath
foreach ($flag in $Target.flags.PSObject.Properties) {
  $line = $cache | Where-Object { $_ -match "^$($flag.Name):[A-Z]+=" } | Select-Object -First 1
  if (-not $line -or $line -notmatch "=$($flag.Value)$") {
    throw "The runtime was not built portably ($($flag.Name) is not $($flag.Value)). Delete $BuildDir and build again."
  }
}
$triplet = $cache | Where-Object { $_ -match '^VCPKG_TARGET_TRIPLET:[A-Z]+=' } | Select-Object -First 1
if (-not $triplet -or ($triplet -replace '^[^=]*=', '') -ne $Target.vcpkg.triplet) {
  throw "The build did not use the vcpkg triplet $($Target.vcpkg.triplet)"
}

$compilerFile = Get-ChildItem -LiteralPath (Join-Path $BuildDir 'CMakeFiles') -Recurse -Filter 'CMakeCXXCompiler.cmake' | Select-Object -First 1
if (-not $compilerFile) { throw "CMake recorded no C++ compiler in $BuildDir" }
$compilerText = [IO.File]::ReadAllText($compilerFile.FullName)
if ($compilerText -notmatch 'set\(CMAKE_CXX_COMPILER_ID "MSVC"\)') { throw 'The runtime was not built with MSVC' }
if ($compilerText -notmatch 'set\(CMAKE_CXX_COMPILER_VERSION "([\d.]+)"\)') { throw 'CMake recorded no compiler version' }
$msvc = $Matches[1]
Write-Host "[nemo-runtime] Built with MSVC $msvc"

$vs = Get-VisualStudio
$dumpbin = Get-ChildItem -Path (Join-Path $vs 'VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe') | Sort-Object { [version]$_.Directory.Parent.Parent.Parent.Name } | Select-Object -Last 1
if (-not $dumpbin) { throw "dumpbin.exe not found in $vs" }

# Everything a DLL may import: each other, the Visual C++ runtime that ships next to them, the UCRT and system DLLs.
# The worker loads these system DLLs from System32 before the runtime; a new one also goes into SYSTEM_IMPORTS in
# voice/crates/voice-speakers/src/nemo.rs.
$allowed = '^(nemo_speech_asr\.dll|ggml(-[a-z]+)?\.dll|msvcp140\.dll|vcruntime140(_1)?\.dll|kernel32\.dll|advapi32\.dll|dbghelp\.dll|api-ms-win-crt-[a-z0-9-]+\.dll)$'
$bin = Join-Path $BuildDir 'bin'
foreach ($name in $Libraries) {
  $file = Join-Path $bin $name
  if (-not (Test-Path -LiteralPath $file)) { throw "The build made no $name" }
  $imports = @(& $dumpbin.FullName /nologo /dependents $file | Where-Object { $_ -match '^\s+\S+\.dll\s*$' } | ForEach-Object { $_.Trim() })
  if ($LASTEXITCODE -ne 0 -or $imports.Count -eq 0) { throw "dumpbin could not read $file" }
  if ($imports | Where-Object { $_ -match '^vcomp' }) { throw "$name uses OpenMP ($($imports -join ', ')); the runtime must be built without it" }
  $unexpected = @($imports | Where-Object { $_ -notmatch $allowed })
  if ($unexpected.Count -gt 0) { throw "$name imports $($unexpected -join ', '), which the runtime folder doesn't ship" }
  Write-Host "  ${name}: $($imports -join ', ')"
}

# The Visual C++ runtime from the install that compiled the DLLs, newest redistributable version first.
$redist = Get-ChildItem -LiteralPath (Join-Path $vs 'VC\Redist\MSVC') -Directory | Where-Object { $_.Name -match '^\d+(\.\d+){2,3}$' } |
  Sort-Object { [version]$_.Name } | Select-Object -Last 1
if (-not $redist) { throw "No Visual C++ redistributable folder in $vs\VC\Redist\MSVC" }
$crtDir = Get-ChildItem -LiteralPath (Join-Path $redist.FullName 'x64') -Directory -Filter 'Microsoft.VC14*.CRT' | Select-Object -First 1
if (-not $crtDir) { throw "No Microsoft.VC14*.CRT folder in $($redist.FullName)\x64" }
# The toolset 14.N goes with compiler 19.N; its runtime must be at least that new.
$needed = [version]"14.$(([version]$msvc).Minor)"
foreach ($name in $Crt) {
  $file = Join-Path $crtDir.FullName $name
  if (-not (Test-Path -LiteralPath $file)) { throw "$file is missing" }
  $version = [version](Get-FileVersionText $file)
  if ($version -lt $needed) { throw "$name is $version, older than the toolset $needed that built the runtime" }
}

# vcpkg links SentencePiece and what it builds on (protobuf, Abseil, utf8-range) statically into the DLLs. Their
# licenses come along as vcpkg installed them, one file per package; NeMo-Speech.cpp's own notices only name them.
$vcpkgShare = Join-Path $BuildDir "vcpkg_installed\$($Target.vcpkg.triplet)\share"
$vcpkgLicenses = @(Get-ChildItem -LiteralPath $vcpkgShare -Directory | Where-Object {
    $_.Name -notmatch '^vcpkg-' -and (Test-Path -LiteralPath (Join-Path $_.FullName 'copyright'))
  } | Sort-Object Name | ForEach-Object { $_.Name })
if ($vcpkgLicenses -notcontains 'sentencepiece') { throw "vcpkg installed no SentencePiece license in $vcpkgShare" }

$staging = "$Out.staging"
if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
New-Item -ItemType Directory -Force -Path (Join-Path $staging 'licenses\vcpkg') | Out-Null
foreach ($name in $Libraries) { Copy-Item -LiteralPath (Join-Path $bin $name) -Destination $staging }
foreach ($name in $Crt) { Copy-Item -LiteralPath (Join-Path $crtDir.FullName $name) -Destination (Join-Path $staging $name) }
foreach ($name in $Licenses) { Copy-Item -LiteralPath (Join-Path $Source $name) -Destination (Join-Path $staging 'licenses') }
foreach ($name in $OwnLicenses) { Copy-Item -LiteralPath (Join-Path $PSScriptRoot "licenses\$name") -Destination (Join-Path $staging 'licenses') }
foreach ($name in $vcpkgLicenses) {
  Copy-Item -LiteralPath (Join-Path $vcpkgShare "$name\copyright") -Destination (Join-Path $staging "licenses\vcpkg\$name.txt")
}

$files = @()
$licensePaths = @($Licenses + $OwnLicenses | ForEach-Object { "licenses/$_" }) + @($vcpkgLicenses | ForEach-Object { "licenses/vcpkg/$_.txt" })
foreach ($path in @($Libraries + $Crt + $licensePaths)) {
  $file = Join-Path $staging $path
  $entry = [ordered]@{ path = $path; size = (Get-Item -LiteralPath $file).Length; sha256 = (Get-Sha256 $file) }
  if ($Crt -contains $path) { $entry.version = Get-FileVersionText $file }
  $files += New-Object PSObject -Property $entry
}
$runtime = [ordered]@{
  # Format 1 is the build record: one folder, one platform, in the shape pack.ps1 reads.
  format = 1
  target = 'windows-x86_64'
  name = $Manifest.name
  repository = $Manifest.repository
  commit = $commit
  submodules = $Manifest.submodules
  build = $Target.build
  flags = $Target.flags
  vcpkg = $Target.vcpkg
  toolset = [ordered]@{ runner = $Runner; msvc = $msvc }
  cpu = $Target.cpu
  files = $files
}
$json = ($runtime | ConvertTo-Json -Depth 6).Replace("`r`n", "`n")
[IO.File]::WriteAllText((Join-Path $staging 'runtime.json'), $json + "`n", $Utf8)

if (Test-Path -LiteralPath $Out) { Remove-Item -LiteralPath $Out -Recurse -Force }
Move-Item -LiteralPath $staging -Destination $Out
Write-Host "[nemo-runtime] Runtime staged in $Out"
foreach ($entry in $files) { Write-Host ("  {0,12:N0}  {1}  {2}" -f $entry.size, $entry.sha256, $entry.path) }
