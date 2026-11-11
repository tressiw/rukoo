# The NeMo runtime for speaker labels

Voice labels the speakers on a call with NVIDIA Nemotron 3 Diarization, which runs in [NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp). The speaker worker, `rukoo-voice-speakers`, loads it from a `nemo_speech/` folder next to itself: beside the exe on Windows, and in `Rukoo Voice.app/Contents/Resources/` on macOS. Each platform has its own runtime.

Windows on x86-64:

```
nemo_speech\nemo_speech_asr_c.dll, nemo_speech_asr.dll, ggml.dll, ggml-base.dll, ggml-cpu.dll
nemo_speech\msvcp140.dll, vcruntime140.dll, vcruntime140_1.dll
nemo_speech\runtime.json
nemo_speech\licenses\LICENSE, NOTICE, THIRD_PARTY_NOTICES.md, darts-clone.txt
nemo_speech\licenses\vcpkg\abseil.txt, protobuf.txt, sentencepiece.txt, utf8-range.txt
```

The `vcpkg` licenses belong to SentencePiece and the libraries it uses, which vcpkg links statically into `nemo_speech_asr.dll`. NeMo-Speech.cpp's own `THIRD_PARTY_NOTICES.md` names them but doesn't carry their texts. SentencePiece also compiles in Darts-clone, whose BSD license vcpkg doesn't install, so `build.ps1` adds it from `licenses/darts-clone.txt` in this folder.

macOS on Apple Silicon:

```
nemo_speech/libnemo_speech_asr_c.dylib, libnemo_speech_asr.dylib
nemo_speech/libggml.dylib, libggml-base.dylib, libggml-cpu.dylib, libggml-blas.dylib
nemo_speech/runtime.json
nemo_speech/licenses/LICENSE, NOTICE, THIRD_PARTY_NOTICES.md
nemo_speech/licenses/sentencepiece/LICENSE, absl-LICENSE, darts-clone-LICENSE, protobuf-lite-LICENSE
```

There is no vcpkg on macOS and no Visual C++ runtime: the C++ runtime and libSystem come with macOS, `libggml-blas` and `libggml-cpu` import Accelerate, and SentencePiece is built from source by NeMo's own script, which already stages its four licenses. The six dylibs are 3.8 MB in all.

Voice builds this runtime itself, once per pinned commit, because MSVC builds differ between toolsets: Ownkey's copy and Voice's copy of the same commit have different hashes. So do macOS builds from different folders, because the assert strings embed the source path. The Windows build is portable, an AVX2 baseline with FMA, F16C and BMI2 and no OpenMP, so it runs on any processor that has those four, and the Visual C++ runtime ships next to the DLLs. The macOS build is portable the same way: an ARMv8.2-A baseline with dot-product and FP16 arithmetic, which every Apple Silicon CPU has, no Metal and no OpenMP, and `minos 15.0`.

| File | What it is |
|---|---|
| `manifest.json` | The pins: format 2, the commit and submodules every platform shares, and one entry per target (`windows-x86_64`, `macos-aarch64`) with its build, flags, toolset and cpu. Once a target is published, its entry also has the archive's URL, size and SHA-256 and the size and SHA-256 of every file in it. |
| `build.ps1` | Builds the Windows runtime folder from source. |
| `build-macos.sh` | Builds the macOS runtime folder from source. |
| `macos.patch` | The three upstream build-file edits the macOS build needs, applied by `build-macos.sh`. |
| `pack.ps1` | Checks a built Windows folder, zips it, and writes the filled-in `manifest.json` and the release notes. |
| `licenses/darts-clone.txt` | Darts-clone's license from SentencePiece 0.2.1, unchanged, for the Windows runtime's `licenses\`. |

Who reads `manifest.json`: voice-core compiles it in to check Voice's own `nemo_speech/` folder for its own platform — next to the exe, or in the bundle's `Contents/Resources` — before it offers speaker labels; `voice/scripts/package.ps1` downloads the Windows archive, checks every hash and puts the folder into the installer and the portable zip; `voice/scripts/speaker-assets.ps1` and `speaker-assets.mjs` fetch a published runtime for the tests.

## The manifest format

`manifest.json` is format 2. The top holds what every platform shares: `name`, `repository`, `commit` and `submodules`. Under `targets` each platform has an entry with its own `build`, `flags`, `toolset` and `cpu`, and, once published, `archive` and `files`.

Until a target's runtime is published its `archive` is `null` and its `files` empty. Then Voice's own `nemo_speech/` never verifies for that platform, so a development copy uses `RUKOO_VOICE_NEMO_DIR`, the runtime tests skip, and `package.ps1` stops with "Publish the NeMo runtime first". The macOS entry is in that state today.

`build.ps1` and `build-macos.sh` write a `runtime.json` into the folder they build. That file is the opposite way round: format 1, the record of one build, with `target` naming the manifest entry it belongs to and the size and SHA-256 of every file. `pack.ps1` and the asset scripts check a folder against it.

Format 1 manifests are still read, as the Windows entry alone, so a test package made from an artifact older than the per-target format keeps verifying on Windows.

## Building it on Windows

You need Git, Visual Studio 2022 with the C++ workload and Python's `py.exe`. CMake and Ninja are installed into a Python environment in the build folder when they are missing.

```powershell
powershell -File voice\nemo-runtime\build.ps1 -Out tmp\nemo\nemo_speech -Jobs 6
```

It clones NeMo-Speech.cpp next to `-Out` unless you pass `-Source`, seeds the portable flags into `CMakeCache.txt`, runs NeMo's `scripts\windows\build.ps1 -Backend cpu -Profile asr`, reads the flags back, checks every DLL's imports with `dumpbin`, copies the Visual C++ runtime from the same Visual Studio install, and writes `runtime.json`. NeMo's script builds SentencePiece with vcpkg under `%LOCALAPPDATA%\NeMoSpeech` the first time, which takes about half an hour; later builds take a few minutes.

To check a build, label the AMI excerpt with it. The commands run from the repository root:

```powershell
powershell -File voice\scripts\speaker-assets.ps1 -Out tmp\speaker-assets -Model -Ami
cargo build --manifest-path voice\Cargo.toml --config voice\.cargo\config.toml --release -p voice-speakers
voice\target\release\rukoo-voice-speakers.exe --self-test --runtime tmp\nemo\nemo_speech --model tmp\speaker-assets\nemotron\Nemotron-3-Diarization.q8_0.gguf --wav tmp\speaker-assets\ami\ami_en2002d_2132.wav
```

Cargo reads `voice\.cargo\config.toml` only when it runs inside `voice\`, so from the root `--config` passes it in. That file links the C runtime into the worker statically, as in the worker Voice ships. Without it the worker imports the Visual C++ runtime itself, and the runtime's DLLs bind to that copy instead of the one in `nemo_speech\`. With `CARGO_TARGET_DIR` set, the exe is under that folder instead of `voice\target`. The self-test should report at least 3 speakers and about 6,000 frames.

## Building it on macOS

You need macOS 15 or later on Apple Silicon, Xcode or its Command Line Tools, git and python3. CMake and Ninja are installed into a Python virtual environment in the build folder when they are missing, as on Windows; no Homebrew and no administrator rights are needed. From the repository root:

```bash
voice/nemo-runtime/build-macos.sh --out tmp/nemo/nemo_speech
```

It refuses a machine that isn't `arm64`, because an Intel build would quietly produce x86_64 dylibs. It clones NeMo-Speech.cpp next to `--out` unless you pass `--source`, checks out the pinned commit and the ggml and llama.cpp submodules from the manifest, and applies `macos.patch`. Then it builds SentencePiece as a static archive with NeMo's own script, configures the `cpu-asr` preset with the manifest's flags, reads them back from `CMakeCache.txt` as `build.ps1` does, and stages the flat dylibs. Finally it checks every one: arm64, `minos 15.0`, and importing only libSystem, libc++, Accelerate and the other dylibs of the folder. A build takes under a minute on an M4.

The first run on a machine whose Command Line Tools SDK is newer than Xcode's linker fails every link with `tapi error: malformed file`; the script probes `clang` and falls back to the SDK inside Xcode by exporting `SDKROOT`, which changes nothing system-wide.

Then label the excerpt, from `voice/`:

```bash
node scripts/speaker-assets.mjs --out tmp/speaker-assets --model --ami
cargo build --release -p voice-speakers
target/release/rukoo-voice-speakers --self-test --runtime tmp/nemo/nemo_speech --model tmp/speaker-assets/nemotron/Nemotron-3-Diarization.q8_0.gguf --wav tmp/speaker-assets/ami/ami_en2002d_2132.wav
```

The self-test should report at least 3 speakers and about 6,000 frames; on an M4 it runs at about 26 times real time. `node scripts/speaker-assets.mjs --out tmp/speaker-assets --nemo-dir tmp/nemo/nemo_speech` checks a local build against the manifest and prints the three variables the runtime tests read, which works before the macOS runtime is published.

## Signing it on macOS

A hardened program refuses to load a dylib that another team signed. Ad-hoc signatures have no Team ID, so macOS derives one from each signature and every ad-hoc signature is a different team: `dlopen` fails with "mapping process and mapped file (non-platform) have different Team IDs". That is why `rukoo-voice-speakers` carries `com.apple.security.cs.disable-library-validation`, and nothing else in Voice does. Packaging signs the six dylibs before the worker — ad-hoc (`codesign -s - --force -o runtime`), or with the Developer ID when there is one — then the worker, then the app, which is also the order `codesign --verify --deep --strict` expects. A hardened process also refuses a relative `dlopen` path, which the worker never uses: it resolves the folder to an absolute path first.

The folder ships in `Rukoo Voice.app/Contents/Resources/nemo_speech/`, not under `Contents/Frameworks/`: codesign reads a folder under `Frameworks` as nested code and refuses the licences and `runtime.json`, which aren't code. Voice's lookup knows that path (`voice-core`'s `speakers::runtime`), and still tries `nemo_speech/` beside the exe for an unbundled build.

The runtime folder as `build-macos.sh` writes it is unsigned; signing changes its hashes, so it happens at packaging time and the manifest pins the signed files.

## The workflow

`.github/workflows/voice-nemo-runtime.yml` builds the runtime whenever a push or pull request changes this folder or the workflow, on any branch, and on a manual dispatch. It has a job per platform:

- `build` on `windows-2025` runs `build.ps1`, builds the worker, runs the self-test on the AMI excerpt (at least 3 speakers and 5,900 frames), runs `pack.ps1` and uploads the artifact `nemo-runtime-97a15af-1`: the zip, `manifest.json`, `notes.md` and `self-test.json`.
- `build-macos` on `macos-15` runs `build-macos.sh` from a fixed checkout path (`$RUNNER_TEMP/nemo/NeMo-Speech.cpp`, because the source path ends up in the dylibs), builds the worker, fetches the model and the AMI excerpt with `speaker-assets.mjs`, runs the same self-test against the fresh runtime and uploads the runtime folder as `nemo-runtime-97a15af-1-macos-arm64`. It uploads no manifest and publishes nothing.

Neither job publishes anything.

## Publishing

Publishing needs the maintainer's approval. It makes the runtime a prerelease of this repository, under a tag that neither update check reads: Rukoo's reads `v` tags and Voice's reads `voice-v` tags, so never tag it either way.

1. Take the artifact of a successful workflow run on the branch that changes the runtime:
   ```powershell
   gh run download <run id> -n nemo-runtime-97a15af-1 -D tmp\nemo-release
   ```
2. Read `self-test.json` and `notes.md`.
3. Publish the zip:
   ```powershell
   gh release create nemo-runtime-97a15af-1 tmp\nemo-release\nemo-runtime-97a15af-1-win-x64.zip --prerelease --latest=false --title "NeMo-Speech.cpp runtime for Rukoo Voice" --notes-file tmp\nemo-release\notes.md
   ```
4. Copy `tmp\nemo-release\manifest.json` over `voice/nemo-runtime/manifest.json` and commit it on the same branch. From then on, CI fetches the runtime and runs the runtime tests instead of skipping them, and a run that couldn't fetch an asset fails rather than passing without them.

The macOS runtime is published under the same tag, from the `-macos-arm64` artifact. That needs a packaging step this folder doesn't have yet: sign the six dylibs, zip the folder's files at the top of `nemo-runtime-97a15af-1-macos-arm64.zip`, and write the `macos-aarch64` entry with the archive's URL, size and SHA-256 and every file's hash, leaving the `windows-x86_64` entry as it is. `pack.ps1` does that for Windows only; adding `pack-macos.mjs` (or teaching the macOS job to pack) belongs with the macOS packaging work.

A new runtime, for a new NeMo-Speech.cpp commit or a new toolset, gets a new name: change `name` and the pins in `manifest.json`, set both targets' `archive` back to `null` and `files` to `[]`, update `NEMO_COMMIT` and `RUNTIME_NAME` in the workflow, and publish under the new name. A published release is never replaced: `package.ps1` on older commits downloads it and checks it against the hashes those commits pinned.
