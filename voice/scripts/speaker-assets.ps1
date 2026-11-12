<#
.SYNOPSIS
Fetches what the speaker label tests need: the NeMo runtime, the Nemotron model and the AMI meeting excerpt.

.DESCRIPTION
Downloads each file once into -Out and checks its size and SHA-256 against a pin; a file already there with the right
hash is kept. Nothing is read from or written to Ownkey's folders.

  <Out>\nemo_speech\                              -Runtime: the published NeMo-Speech.cpp runtime of
                                                  voice/nemo-runtime/manifest.json, every file checked
  <Out>\nemotron\Nemotron-3-Diarization.q8_0.gguf -Model: NVIDIA Nemotron 3 Diarization, the pin of
                                                  voice-core's ModelSpec::nemotron()
  <Out>\ami\                                      -Ami: the three files of voice/fixtures/ami/manifest.json

Without a switch it fetches all three. Until the runtime is published, its manifest has no archive and -Runtime says
so and skips it. At the end it prints the values for RUKOO_VOICE_TEST_NEMO_DIR, RUKOO_VOICE_TEST_NEMOTRON and
RUKOO_VOICE_TEST_AMI, the variables the runtime tests read.

.EXAMPLE
powershell -File voice\scripts\speaker-assets.ps1 -Out tmp\speaker-assets -Model -Ami
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Out,
  [switch]$Runtime,
  [switch]$Model,
  [switch]$Ami
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0
# Windows PowerShell's progress bar slows Invoke-WebRequest down to a crawl, and its default TLS can be too old.
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

if (-not ($Runtime -or $Model -or $Ami)) { $Runtime = $true; $Model = $true; $Ami = $true }

$Voice = Split-Path -Parent $PSScriptRoot
if (-not [IO.Path]::IsPathRooted($Out)) { $Out = Join-Path (Get-Location).Path $Out }
$Out = [IO.Path]::GetFullPath($Out)

# The pin of ModelSpec::nemotron() in voice/crates/voice-core/src/local/registry.rs. voice-speakers' tests/assets.rs
# checks that this script carries the same URL, size and SHA-256.
$NemotronUrl = 'https://huggingface.co/nvidia/Nemotron-3-Diarization/resolve/f667ed73aee57d40cc39428eb768b4fd87a0a29e/Nemotron-3-Diarization.q8_0.gguf'
$NemotronSize = 107012128
$NemotronSha256 = '08456d9e22cd9a323c0364d98375f3746d6e68507ebb705cd46438c534c7a3a1'

function Get-Sha256 {
  param([string]$Path)
  return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Test-Pinned {
  param([string]$Path, [long]$Size, [string]$Sha256)
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
  if ((Get-Item -LiteralPath $Path).Length -ne $Size) { return $false }
  return (Get-Sha256 $Path) -eq $Sha256.ToLowerInvariant()
}

# Downloads $Url to $Path unless a file with the pinned size and hash is already there. The download lands next to
# $Path first and replaces it only once it checks out.
function Get-Pinned {
  param([string]$Url, [string]$Path, [long]$Size, [string]$Sha256)
  $name = Split-Path -Leaf $Path
  if (Test-Pinned $Path $Size $Sha256) {
    Write-Host "  $name is there"
    return
  }
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
  $part = "$Path.part"
  for ($attempt = 1; ; $attempt++) {
    Write-Host ("  {0}: downloading {1:N0} bytes" -f $name, $Size)
    try {
      Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $part
      break
    } catch {
      if ($attempt -ge 3) { throw "Could not download ${Url}: $($_.Exception.Message)" }
      Start-Sleep -Seconds (5 * $attempt)
    }
  }
  $got = (Get-Item -LiteralPath $part).Length
  if ($got -ne $Size) {
    Remove-Item -LiteralPath $part -Force
    throw "$Url has $got bytes instead of $Size"
  }
  $hash = Get-Sha256 $part
  if ($hash -ne $Sha256.ToLowerInvariant()) {
    Remove-Item -LiteralPath $part -Force
    throw "$Url has SHA-256 $hash instead of $Sha256"
  }
  if (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Force }
  Move-Item -LiteralPath $part -Destination $Path
}

# A runtime folder checks out when every manifest file has its size and hash, runtime.json names the same build, and
# nothing else is in it.
function Test-RuntimeFolder {
  param($Manifest, [string]$Dir)
  if (-not (Test-Path -LiteralPath (Join-Path $Dir 'runtime.json'))) { return 'runtime.json is missing' }
  $runtime = Get-Content -LiteralPath (Join-Path $Dir 'runtime.json') -Raw | ConvertFrom-Json
  if ($runtime.name -ne $Manifest.name -or $runtime.commit -ne $Manifest.commit) { return "runtime.json describes $($runtime.name)" }
  $expected = @('runtime.json')
  foreach ($file in $Manifest.files) {
    $expected += $file.path
    if (-not (Test-Pinned (Join-Path $Dir $file.path) $file.size $file.sha256)) { return "$($file.path) is missing or differs" }
  }
  $present = @(Get-ChildItem -LiteralPath $Dir -Recurse -File | ForEach-Object { $_.FullName.Substring($Dir.Length + 1).Replace('\', '/') })
  $extra = @($present | Where-Object { $expected -notcontains $_ })
  if ($extra.Count -gt 0) { return "it also holds $($extra -join ', ')" }
  return $null
}

$variables = [ordered]@{}

if ($Runtime) {
  $manifest = Get-Content -LiteralPath (Join-Path $Voice 'nemo-runtime\manifest.json') -Raw | ConvertFrom-Json
  # Format 2 keeps one entry per platform; this script fetches the Windows one for the tests here.
  $target = $manifest.targets.'windows-x86_64'
  if (-not $target) { throw 'voice/nemo-runtime/manifest.json pins no windows-x86_64 runtime' }
  foreach ($property in $target.PSObject.Properties) {
    $manifest = $manifest | Add-Member -NotePropertyName $property.Name -NotePropertyValue $property.Value -Force -PassThru
  }
  Write-Host "Runtime $($manifest.name)"
  if (-not $manifest.archive) {
    Write-Host '  not published yet: voice/nemo-runtime/manifest.json has no archive, so the runtime is skipped'
  } else {
    $dir = Join-Path $Out 'nemo_speech'
    if ((Test-Path -LiteralPath $dir) -and -not (Test-RuntimeFolder $manifest $dir)) {
      Write-Host '  nemo_speech is there'
    } else {
      $zip = Join-Path $Out (Split-Path -Leaf ([uri]$manifest.archive.url).AbsolutePath)
      Get-Pinned $manifest.archive.url $zip $manifest.archive.size $manifest.archive.sha256
      $staging = "$dir.staging"
      if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
      Expand-Archive -LiteralPath $zip -DestinationPath $staging
      $problem = Test-RuntimeFolder $manifest $staging
      if ($problem) { throw "The runtime archive doesn't match voice/nemo-runtime/manifest.json: $problem" }
      if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
      Move-Item -LiteralPath $staging -Destination $dir
      Write-Host '  nemo_speech checked'
    }
    $variables['RUKOO_VOICE_TEST_NEMO_DIR'] = $dir
  }
}

if ($Model) {
  Write-Host 'Nemotron 3 Diarization'
  $gguf = Join-Path $Out 'nemotron\Nemotron-3-Diarization.q8_0.gguf'
  Get-Pinned $NemotronUrl $gguf $NemotronSize $NemotronSha256
  $variables['RUKOO_VOICE_TEST_NEMOTRON'] = $gguf
}

if ($Ami) {
  $fixtures = Get-Content -LiteralPath (Join-Path $Voice 'fixtures\ami\manifest.json') -Raw | ConvertFrom-Json
  Write-Host 'AMI meeting excerpt'
  $dir = Join-Path $Out 'ami'
  foreach ($file in $fixtures.files) {
    Get-Pinned ($fixtures.source + $file.name) (Join-Path $dir $file.name) $file.size $file.sha256
  }
  $variables['RUKOO_VOICE_TEST_AMI'] = $dir
}

Write-Host ''
foreach ($name in $variables.Keys) { Write-Host "$name=$($variables[$name])" }
