# Third-party notices for Rukoo Voice

Rukoo Voice is released under the MIT license, Copyright (c) 2026 Bart van der Meeren. Its Windows installer and portable zip, and its macOS app (`Rukoo Voice.app`, in the DMG and the zip), hold five programs: `rukoo-voice` (the app), `rukoo-voice-recorder` (the meeting recorder), `rukoo-voice-local` (the local speech worker), `rukoo-voice-speakers` (the speaker worker) and `rukoo-voice-screen` (the screen recorder), plus the speaker label runtime in `nemo_speech`. On Windows the programs have an `.exe` extension and the runtime folder is `nemo_speech\`; on macOS the programs are in `Contents/MacOS` and the runtime is in `Contents/Resources/nemo_speech/`. This file lists the work of others that is built into them, and ships next to them on Windows and in `Contents/Resources` on macOS.

`THIRD_PARTY_LICENSES.txt`, which ships alongside, holds the license files of every Rust crate that platform's build contains, as they appear in each crate's source package, and the license files in `nemo_speech\licenses\` (`nemo_speech/licenses/` on macOS). `voice/scripts/package.ps1` writes it for the Windows installer and `voice/scripts/package-macos.mjs` for the macOS app, both through `voice/scripts/notices.mjs`.

## Speech recognition in rukoo-voice-local

The local speech worker links the prebuilt static library of [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) v2.0.1, through the `sherpa-onnx` and `sherpa-onnx-sys` crates: `sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib` on Windows and `sherpa-onnx-v2.0.1-osx-arm64-static-lib` on macOS. Both link the same eleven libraries, and both contain:

- sherpa-onnx, by the Next-gen Kaldi team (k2-fsa): Apache License 2.0.
- [ONNX Runtime](https://github.com/microsoft/onnxruntime): MIT license, Copyright (c) Microsoft Corporation. The license text is below. ONNX Runtime's own notices for the code it includes are in its [ThirdPartyNotices.txt](https://github.com/microsoft/onnxruntime/blob/main/ThirdPartyNotices.txt).
- [kaldi-native-fbank](https://github.com/csukuangfj/kaldi-native-fbank), [kaldi-decoder](https://github.com/k2-fsa/kaldi-decoder) and [kaldifst](https://github.com/k2-fsa/kaldifst), which build on [Kaldi](https://github.com/kaldi-asr/kaldi): Apache License 2.0.
- [OpenFst](https://www.openfst.org): Apache License 2.0, Copyright Google.
- [simple-sentencepiece](https://github.com/pkufool/simple-sentencepiece): Apache License 2.0.
- [KISS FFT](https://github.com/mborgerding/kissfft): BSD 3-Clause license, Copyright (c) 2003-2010 Mark Borgerding.
- [utf8proc](https://github.com/JuliaStrings/utf8proc): MIT ("expat") license, Copyright (c) 2014-2021 Steven G. Johnson, Jiahao Chen, Tony Kelman, Jonas Fonseca and other contributors, Copyright (c) 2009 Public Software Group e. V., Berlin, Germany; its Unicode data is under the Unicode license.

The Apache License 2.0 text is in `THIRD_PARTY_LICENSES.txt` under `sherpa-onnx-sys`.

The static library is built without text-to-speech, so it holds no eSpeak NG or Piper code.

### ONNX Runtime license

```text
MIT License

Copyright (c) Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Speaker labels: nemo_speech and rukoo-voice-speakers

The speaker worker, `rukoo-voice-speakers`, is a Rust program like the other four; its crates are in the list below. On Windows it loads the speaker label runtime from the `nemo_speech\` folder next to it, on macOS from `Contents/Resources/nemo_speech/`. Voice builds that folder for each platform from [NVIDIA NeMo-Speech.cpp](https://github.com/NVIDIA/NeMo-Speech.cpp) at commit `97a15afa5caa9bce5baaa86c1184103877af4101`, with the CPU backend and no changes to the source (`voice/nemo-runtime/`). `runtime.json` in the folder names the build and the SHA-256 of every file. The Windows folder (`voice/nemo-runtime/build.ps1`) holds:

- NeMo-Speech.cpp, in `nemo_speech_asr_c.dll` and `nemo_speech_asr.dll`: Apache License 2.0, Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. Parts of its runtime derive from [parakeet.cpp](https://github.com/jason-ni/parakeet.cpp), Copyright (c) 2025 Jason Ni, under the MIT license. Its `LICENSE`, `NOTICE` and `THIRD_PARTY_NOTICES.md` are in `nemo_speech\licenses\`.
- [ggml](https://github.com/ggml-org/ggml), in `ggml.dll`, `ggml-base.dll` and `ggml-cpu.dll`: MIT license, Copyright (c) 2023-2026 The ggml authors. The license text is in NeMo-Speech.cpp's `THIRD_PARTY_NOTICES.md`.
- [SentencePiece](https://github.com/google/sentencepiece), linked into `nemo_speech_asr.dll` by vcpkg together with the libraries it uses: SentencePiece and [Abseil](https://github.com/abseil/abseil-cpp) under the Apache License 2.0, [Protocol Buffers](https://github.com/protocolbuffers/protobuf) under the BSD 3-Clause license and utf8-range, which comes with Protocol Buffers, under the MIT license. Their license files, as vcpkg installs them, are in `nemo_speech\licenses\vcpkg\`. SentencePiece compiles in [Darts-clone](https://github.com/s-yata/darts-clone): BSD 2-Clause license, Copyright (c) 2008-2011, Susumu Yata, in `nemo_speech\licenses\darts-clone.txt`.
- `msvcp140.dll`, `vcruntime140.dll` and `vcruntime140_1.dll`: the Microsoft Visual C++ runtime, which the NeMo DLLs need. They are unmodified copies from the Visual Studio build tools that compiled the runtime, redistributed with the program as the Visual Studio license terms allow. `runtime.json` records their versions.

The macOS folder, for Apple Silicon (`voice/nemo-runtime/build-macos.sh`), is the same commit with the same CPU backend and three small build-file edits (`voice/nemo-runtime/macos.patch`). It holds:

- NeMo-Speech.cpp, in `libnemo_speech_asr_c.dylib` and `libnemo_speech_asr.dylib`: Apache License 2.0 and the MIT-licensed parakeet.cpp parts, as above. Its `LICENSE`, `NOTICE` and `THIRD_PARTY_NOTICES.md` are in `nemo_speech/licenses/`.
- ggml, in `libggml.dylib`, `libggml-base.dylib`, `libggml-cpu.dylib` and `libggml-blas.dylib`: MIT license, as above. `libggml-blas.dylib` and `libggml-cpu.dylib` call Accelerate, which comes with macOS.
- SentencePiece, which NeMo's own script builds from source as a static library and links into `libnemo_speech_asr.dylib`, together with the code it compiles in: SentencePiece and Abseil under the Apache License 2.0, the protobuf-lite runtime under the BSD 3-Clause license and Darts-clone under the BSD 2-Clause license. Their license files are in `nemo_speech/licenses/sentencepiece/` (`LICENSE`, `absl-LICENSE`, `protobuf-lite-LICENSE` and `darts-clone-LICENSE`). There is no vcpkg on macOS, so there is no utf8-range, and the C++ runtime comes with macOS, so no Visual C++ runtime is shipped.

`THIRD_PARTY_LICENSES.txt` repeats every file in `nemo_speech\licenses\` (`nemo_speech/licenses/` on macOS). NeMo-Speech.cpp's own command-line program isn't included, so neither is the miniaudio code from llama.cpp that only that program compiles in.

## Speech models are not included

None of the installer, the zips and the DMG contains a speech model. The Orukeet model ([oruk/orukeet](https://huggingface.co/oruk/orukeet)) is licensed under [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/). Voice downloads it into its own data folder when you ask for it, or, on Windows, uses the copy Ownkey already downloaded to `%LOCALAPPDATA%\Ownkey\models`, which it only reads. Every model Voice downloads comes with the license on its model page.

Speaker labels use NVIDIA's Nemotron 3 Diarization weights ([nvidia/Nemotron-3-Diarization](https://huggingface.co/nvidia/Nemotron-3-Diarization)), licensed under the OpenMDW License 1.1. On Windows Voice uses the copy Ownkey already downloaded, which it only reads, or downloads the 107 MB file into its own data folder when you ask for it; on macOS it always downloads it.

## The app: rukoo-voice

- [Tauri](https://tauri.app) and the crates it brings: MIT OR Apache-2.0. They are in the crate list below.
- The Microsoft WebView2 loader (`WebView2LoaderStatic.lib` from the WebView2 SDK), linked through the `webview2-com-sys` crate. Its license:

  ```text
  Copyright (C) Microsoft Corporation. All rights reserved.

  Redistribution and use in source and binary forms, with or without
  modification, are permitted provided that the following conditions are
  met:

     * Redistributions of source code must retain the above copyright
  notice, this list of conditions and the following disclaimer.
     * Redistributions in binary form must reproduce the above
  copyright notice, this list of conditions and the following disclaimer
  in the documentation and/or other materials provided with the
  distribution.
     * The name of Microsoft Corporation, or the names of its contributors
  may not be used to endorse or promote products derived from this
  software without specific prior written permission.

  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
  "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
  LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
  A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
  OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
  LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
  DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
  THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
  (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
  OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
  ```

- [SQLite](https://sqlite.org), compiled in through `libsqlite3-sys`: public domain.
- [Inter](https://github.com/rsms/inter), the typeface of the pill and Settings: SIL Open Font License 1.1, Copyright 2016 The Inter Project Authors. The app carries the font together with its license (`voice/ui/fonts/Inter-LICENSE.txt`), and `THIRD_PARTY_LICENSES.txt` repeats that license.
- The Rukoo Voice icon is drawn from Rukoo's own pigeon, which is part of this repository.

## The screen recorder: rukoo-voice-screen

The screen recorder uses Windows' own Windows.Graphics.Capture, Media Foundation (for the H.264 encoder and the MP4 files) and Windows.Media.Ocr, which come with Windows. On macOS it uses ScreenCaptureKit, VideoToolbox and Vision, which come with macOS. It ships none of their code.

## Rust crates

The Rust crates built into the five programs, by license. The list covers the normal dependencies for Windows x64 (`x86_64-pc-windows-msvc`) and for macOS on Apple Silicon (`aarch64-apple-darwin`), and leaves out the procedural macros, which run only while compiling, and Voice's own crates. A crate only one build contains says so; every other crate is in both. `voice/scripts/notices.mjs` writes the list.

<!-- rust-crates:start: written by voice/scripts/notices.mjs --update (or package.ps1 -UpdateNotices); package.ps1 and package-macos.mjs stop when this list is out of date -->

255 crates: 189 in both builds, 22 only in the Windows build and 44 only in the macOS build.

### 0BSD OR Apache-2.0 OR MIT

- adler2 2.0.1

### Apache-2.0

- cpal 0.18.2
- sherpa-onnx 2.0.1
- sherpa-onnx-sys 2.0.1
- tao 0.37.2

### Apache-2.0 AND ISC

- ring 0.17.14

### Apache-2.0 AND MIT

- dpi 0.1.2

### Apache-2.0 OR BSD-2-Clause OR MIT

- mach2 0.6.0 (macOS only)

### Apache-2.0 OR CC0-1.0 OR MIT-0

- dunce 1.0.5

### Apache-2.0 OR ISC OR MIT

- rustls 0.23.46

### Apache-2.0 OR MIT

- aead 0.6.1
- anyhow 1.0.104
- base64 0.22.1
- base64 0.23.1
- bitflags 2.13.2
- block-buffer 0.10.4
- block-buffer 0.12.1
- bzip2 0.6.1
- cfg-if 1.0.5
- chacha20 0.10.2
- chacha20poly1305 0.11.0
- cipher 0.5.2
- cmov 0.5.4
- const-oid 0.10.2
- cookie 0.18.2
- core-foundation 0.10.1 (macOS only)
- core-foundation-sys 0.8.7 (macOS only)
- core-graphics 0.25.0 (macOS only)
- core-graphics-types 0.2.0 (macOS only)
- coreaudio-rs 0.14.2 (macOS only)
- cpufeatures 0.2.17
- cpufeatures 0.3.1
- crc32fast 1.5.3
- crossbeam-channel 0.5.17
- crossbeam-utils 0.8.23
- crypto-common 0.1.7
- crypto-common 0.2.2
- ctor 1.0.13
- ctutils 0.4.7
- dasp_sample 0.11.0
- deranged 0.5.9
- digest 0.10.7
- digest 0.11.3
- dirs 7.0.0
- dirs-sys 0.5.0
- embed_plist 1.2.2 (macOS only)
- equivalent 1.0.3
- erased-serde 0.4.11
- fallible-iterator 0.3.0
- fallible-streaming-iterator 0.1.9
- fdeflate 0.3.7 (macOS only)
- filetime 0.2.29
- flate2 1.1.10
- fnv 1.0.7
- foreign-types 0.5.0 (macOS only)
- foreign-types-shared 0.3.1 (macOS only)
- form_urlencoded 1.2.2
- futures-core 0.3.34
- futures-sink 0.3.34
- futures-task 0.3.34
- futures-util 0.3.34
- getrandom 0.2.17
- getrandom 0.3.4
- getrandom 0.4.3
- glob 0.3.4
- hashbrown 0.17.1
- hashlink 0.12.2
- heck 0.5.0
- hmac 0.13.0
- http 1.5.0
- httparse 1.10.1
- hybrid-array 0.4.15
- idna 1.1.0
- idna_adapter 1.2.2
- indexmap 2.14.2
- inout 0.2.5
- instant-xml 0.7.7
- itoa 1.0.18
- json-patch 4.2.0
- jsonptr 0.7.1
- keyboard-types 0.8.3
- libc 0.2.191
- lock_api 0.4.14 (Windows only)
- log 0.4.34
- md-5 0.11.0
- mime 0.3.17
- muda 0.20.0
- num-conv 0.2.2
- once_cell 1.21.4
- parking_lot 0.12.5 (Windows only)
- parking_lot_core 0.9.12 (Windows only)
- percent-encoding 2.3.2
- pin-project-lite 0.2.17
- png 0.18.1 (macOS only)
- poly1305 0.9.1
- powerfmt 0.2.1
- rand 0.10.3
- rand_core 0.10.1
- regex 1.13.1
- regex-automata 0.4.18
- regex-syntax 0.8.11
- rustls-pki-types 1.15.1
- scopeguard 1.2.0 (Windows only)
- security-framework 3.7.0 (macOS only)
- security-framework-sys 2.17.0 (macOS only)
- semver 1.0.28
- serde 1.0.229
- serde-untagged 0.1.9
- serde_core 1.0.229
- serde_json 1.0.154
- serde_spanned 1.1.2
- serde_with 3.24.0
- serialize-to-javascript 0.1.2
- sha1 0.11.0
- sha2 0.10.9
- sha2 0.11.1
- siphasher 1.0.4
- smallvec 1.16.4
- socket2 0.6.5
- softbuffer 0.4.8 (Windows only)
- stable_deref_trait 1.2.1
- tar 0.4.46
- tauri 2.12.5
- tauri-runtime 2.12.2
- tauri-runtime-wry 2.12.3
- tauri-utils 2.10.2
- thiserror 2.0.21
- time 0.3.55
- time-core 0.1.9
- toml 1.1.8+spec-1.1.0
- toml_datetime 1.1.2+spec-1.1.0
- toml_parser 1.1.5+spec-1.1.0
- toml_writer 1.1.3+spec-1.1.0
- tray-icon 0.25.1
- tungstenite 0.30.0
- typeid 1.0.3
- typenum 1.20.1
- unicase 2.10.0
- unicode-segmentation 1.13.3 (Windows only)
- universal-hash 0.6.1
- ureq 3.5.0
- ureq-proto 0.6.5
- url 2.5.8
- utf8-zero 0.8.1
- utf8_iter 1.1.0
- utf_types 1.0.0
- uuid 1.29.0
- web-time 1.1.0
- window-vibrancy 0.8.1
- windows 0.62.2 (Windows only)
- windows-collections 0.3.2 (Windows only)
- windows-core 0.62.2 (Windows only)
- windows-future 0.3.2 (Windows only)
- windows-link 0.2.1 (Windows only)
- windows-numerics 0.3.1 (Windows only)
- windows-result 0.4.1 (Windows only)
- windows-strings 0.5.1 (Windows only)
- windows-sys 0.61.2 (Windows only)
- windows-threading 0.2.1 (Windows only)
- windows-version 0.1.7 (Windows only)
- wry 0.57.1
- xmlparser 0.13.6
- zeroize 1.9.1

### Apache-2.0 OR MIT OR Zlib

- dispatch2 0.3.1 (macOS only)
- miniz_oxide 0.8.9 (macOS only)
- miniz_oxide 0.9.1
- objc2-app-kit 0.3.2 (macOS only)
- objc2-application-services 0.3.2 (macOS only)
- objc2-audio-toolbox 0.3.2 (macOS only)
- objc2-av-foundation 0.3.2 (macOS only)
- objc2-core-audio 0.3.2 (macOS only)
- objc2-core-audio-types 0.3.2 (macOS only)
- objc2-core-foundation 0.3.2 (macOS only)
- objc2-core-graphics 0.3.2 (macOS only)
- objc2-core-image 0.3.2 (macOS only)
- objc2-core-media 0.3.2 (macOS only)
- objc2-core-ml 0.3.2 (macOS only)
- objc2-core-text 0.3.2 (macOS only)
- objc2-core-video 0.3.2 (macOS only)
- objc2-exception-helper 0.1.1 (macOS only)
- objc2-image-io 0.3.2 (macOS only)
- objc2-io-kit 0.3.2 (macOS only)
- objc2-metal 0.3.2 (macOS only)
- objc2-quartz-core 0.3.2 (macOS only)
- objc2-screen-capture-kit 0.3.2 (macOS only)
- objc2-security 0.3.2 (macOS only)
- objc2-service-management 0.3.2 (macOS only)
- objc2-user-notifications 0.3.2 (macOS only)
- objc2-video-toolbox 0.3.2 (macOS only)
- objc2-vision 0.3.2 (macOS only)
- objc2-web-kit 0.3.2 (macOS only)
- raw-window-handle 0.6.2

### BSD-2-Clause

- rusty-s3 0.10.0

### BSD-3-Clause

- alloc-no-stdlib 3.0.0
- alloc-stdlib 0.3.0
- subtle 2.6.1

### BSD-3-Clause AND MIT

- brotli 9.1.1

### BSD-3-Clause OR MIT

- brotli-decompressor 6.2.0

### CDLA-Permissive-2.0

- webpki-roots 0.26.11
- webpki-roots 1.0.11

### ISC

- rustls-webpki 0.103.16
- untrusted 0.9.0

### MIT

- block2 0.6.2 (macOS only)
- bytes 1.12.1
- cfb 0.14.0
- data-encoding 2.11.1
- generic-array 0.14.7
- infer 0.22.0
- libsqlite3-sys 0.39.1
- mime_guess 2.0.5
- mio 1.2.4
- objc2 0.6.5 (macOS only)
- objc2-encode 4.1.0 (macOS only)
- objc2-foundation 0.3.2 (macOS only)
- phf 0.13.1
- phf_shared 0.13.1
- plist 1.10.3
- quick-xml 0.42.0
- rusqlite 0.41.1
- simd-adler32 0.3.10
- slab 0.4.12
- tokio 1.54.1
- tokio-tungstenite 0.30.0
- tracing 0.1.44 (Windows only)
- tracing-core 0.1.36 (Windows only)
- urlpattern 0.6.0
- webview2-com 0.39.1 (Windows only)
- webview2-com-sys 0.39.1 (Windows only)
- winnow 1.0.4
- zmij 1.0.23

### MIT OR Unlicense

- aho-corasick 1.1.5
- jiff 0.2.38
- jiff-core 0.1.1
- memchr 2.8.3
- same-file 1.0.6
- walkdir 2.5.0
- winapi-util 0.1.11 (Windows only)

### MPL-2.0

- option-ext 0.2.0

### Unicode-3.0

- icu_collections 2.4.0
- icu_locale_core 2.4.0
- icu_normalizer 2.4.0
- icu_normalizer_data 2.4.0
- icu_properties 2.4.0
- icu_properties_data 2.4.0
- icu_provider 2.4.0
- litemap 0.8.3
- potential_utf 0.1.6
- tinystr 0.8.5
- writeable 0.6.4
- yoke 0.8.4
- zerofrom 0.1.8
- zerotrie 0.2.5
- zerovec 0.11.9

### Zlib

- foldhash 0.2.0

### bzip2-1.0.6

- libbz2-rs-sys 0.2.6

<!-- rust-crates:end -->
