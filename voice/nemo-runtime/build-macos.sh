#!/bin/bash
# Builds the NeMo-Speech.cpp runtime that Rukoo Voice's speaker worker loads on Apple Silicon.
#
# This is the macOS counterpart of voice/nemo-runtime/build.ps1: the same pinned commit and
# submodules from manifest.json, the same CPU-only asr profile, and the same staged nemo_speech/
# folder of dylibs, licences and runtime.json. It is written for a clean macos-15 runner: Xcode (or
# its Command Line Tools), git and python3 are assumed, nothing else. CMake and Ninja are installed
# into a Python virtual environment inside the build folder, as build.ps1 does on Windows, so no
# Homebrew and no administrator rights are needed.
#
# The folder it writes:
#
#   libnemo_speech_asr_c.dylib, libnemo_speech_asr.dylib     NeMo-Speech.cpp, flat names with
#   libggml.dylib, libggml-base.dylib, libggml-cpu.dylib,   @loader_path install names
#   libggml-blas.dylib
#   licenses/LICENSE, NOTICE, THIRD_PARTY_NOTICES.md        NeMo-Speech.cpp's own
#   licenses/sentencepiece/{LICENSE,absl-LICENSE,          what the static SentencePiece build
#     darts-clone-LICENSE,protobuf-lite-LICENSE}            compiles in
#   runtime.json                                            the pins, the flags, the toolset, and
#                                                           the size and SHA-256 of every file
#
# Every dylib is checked after staging: it has to be arm64, report minos 15.0, and import nothing
# but libSystem, libc++, Accelerate and the other dylibs of the folder. Step 5's packaging signs
# them; this script doesn't.
#
# Usage:
#   build-macos.sh --out tmp/nemo/nemo_speech [--source DIR] [--jobs N] [--runner NAME]

set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
MANIFEST=$HERE/manifest.json
PATCH=$HERE/macos.patch
TARGET=macos-aarch64

OUT=
SOURCE=
JOBS=$(sysctl -n hw.ncpu 2>/dev/null || echo 4)
RUNNER=local

usage() {
  sed -n '/^# Usage:/,/^$/p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT=$2; shift 2 ;;
    --source) SOURCE=$2; shift 2 ;;
    --jobs) JOBS=$2; shift 2 ;;
    --runner) RUNNER=$2; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done

[ -n "$OUT" ] || { echo "--out is required" >&2; exit 2; }
absolute() { case "$1" in /*) printf '%s' "$1" ;; *) printf '%s/%s' "$PWD" "$1" ;; esac; }
OUT=$(absolute "$OUT")
[ -n "$SOURCE" ] || SOURCE=$(dirname "$OUT")/NeMo-Speech.cpp
SOURCE=$(absolute "$SOURCE")
WORK=$(dirname "$OUT")
mkdir -p "$WORK"

# Voice ships Apple Silicon only (plan D1); an Intel runner would quietly produce x86_64 dylibs.
if [ "$(uname -m)" != "arm64" ]; then
  echo "[nemo-runtime] this builds the arm64 runtime and needs an Apple Silicon machine, not $(uname -m)" >&2
  exit 1
fi

# The pins, the flags and the deployment target come from manifest.json, as build.ps1 reads them,
# so the manifest stays the one place a runtime is described.
eval "$(python3 - "$MANIFEST" "$TARGET" <<'PY'
import json, shlex, sys
path, target = sys.argv[1], sys.argv[2]
doc = json.load(open(path))
if target not in doc.get("targets", {}):
    sys.exit(f"{path} has no {target} entry")
entry = doc["targets"][target]
q = lambda value: shlex.quote(str(value))
print("NAME=" + q(doc["name"]))
print("REPOSITORY=" + q(doc["repository"]))
print("COMMIT=" + q(doc["commit"]))
print("GGML_PIN=" + q(doc["submodules"]["ggml"]))
print("LLAMA_PIN=" + q(doc["submodules"]["llama.cpp"]))
print("BUILD_TEXT=" + q(entry["build"]))
print("DEPLOYMENT_TARGET=" + q(entry["toolset"]["deploymentTarget"]))
print("FLAG_NAMES=(" + " ".join(q(name) for name in entry["flags"]) + ")")
print("FLAG_VALUES=(" + " ".join(q(value) for value in entry["flags"].values()) + ")")
PY
)"
[ -n "${NAME:-}" ] || { echo "[nemo-runtime] could not read the pins from $MANIFEST" >&2; exit 1; }

echo "[nemo-runtime] $NAME: NeMo-Speech.cpp ${COMMIT:0:7} in $SOURCE"

# --- toolchain ---------------------------------------------------------------------------------
# The SDK and the linker must agree. A machine whose Command Line Tools SDK is newer than Xcode's
# ld fails every link with "tapi error: malformed file"; falling back to the SDK that ships inside
# Xcode fixes it without touching anything system-wide.
probe_cc() {
  local dir sdk=${1:-} status
  dir=$(mktemp -d)
  printf 'int main(void){return 0;}\n' > "$dir/probe.c"
  if [ -n "$sdk" ]; then
    "$CC" -isysroot "$sdk" "$dir/probe.c" -o "$dir/probe" >/dev/null 2>&1
  else
    "$CC" "$dir/probe.c" -o "$dir/probe" >/dev/null 2>&1
  fi
  status=$?
  rm -rf "$dir"
  return $status
}
CC=$(command -v clang || echo /usr/bin/cc)
if ! probe_cc; then
  found=
  for sdk in $(ls -d /Applications/Xcode*.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/*.sdk 2>/dev/null | sort -V | tail -3) \
             "$(xcrun --sdk macosx --show-sdk-path 2>/dev/null)"; do
    [ -d "$sdk" ] || continue
    if probe_cc "$sdk"; then found=$sdk; break; fi
  done
  if [ -z "$found" ]; then
    echo "[nemo-runtime] the C toolchain cannot link a trivial program; fix Xcode/Command Line Tools" >&2
    exit 1
  fi
  echo "[nemo-runtime] the default SDK does not link with this ld; using $found"
  export SDKROOT=$found
fi
export MACOSX_DEPLOYMENT_TARGET=$DEPLOYMENT_TARGET

# --- cmake and ninja ---------------------------------------------------------------------------
tools=$WORK/voice-tools
if ! command -v cmake >/dev/null || ! command -v ninja >/dev/null; then
  if [ ! -x "$tools/bin/cmake" ]; then
    python3=$(command -v python3 || echo /usr/bin/python3)
    echo "[nemo-runtime] installing CMake and Ninja into $tools"
    "$python3" -m venv "$tools"
    "$tools/bin/python" -m pip install --disable-pip-version-check -q cmake ninja
  fi
  export PATH=$tools/bin:$PATH
fi
cmake --version | head -1
ninja --version

# --- source ------------------------------------------------------------------------------------
if [ ! -d "$SOURCE/.git" ]; then
  # An interrupted earlier run may have left an empty folder; git clone wants it absent.
  [ -d "$SOURCE" ] && rmdir "$SOURCE" 2>/dev/null || true
  git clone --quiet --filter=blob:none --no-checkout "$REPOSITORY" "$SOURCE"
fi
if ! git -C "$SOURCE" rev-parse --verify --quiet "$COMMIT^{commit}" >/dev/null; then
  git -C "$SOURCE" fetch --quiet origin "$COMMIT"
fi
git -C "$SOURCE" checkout --quiet --detach "$COMMIT"
# The asr profile reads miniaudio from llama.cpp, so both submodules are needed even though only
# ggml is linked.
git -C "$SOURCE" submodule update --init --depth 1 ggml llama.cpp
[ "$(git -C "$SOURCE/ggml" rev-parse HEAD)" = "$GGML_PIN" ] || { echo "ggml is not at its pin" >&2; exit 1; }
[ "$(git -C "$SOURCE/llama.cpp" rev-parse HEAD)" = "$LLAMA_PIN" ] || { echo "llama.cpp is not at its pin" >&2; exit 1; }

# Upstream's macOS path expects Homebrew's dynamic SentencePiece and its linker options are
# ELF-only; macos.patch carries the three build-file edits, and says what each is for.
if git -C "$SOURCE" apply --check "$PATCH" >/dev/null 2>&1; then
  git -C "$SOURCE" apply "$PATCH"
  echo "[nemo-runtime] applied macos.patch"
elif git -C "$SOURCE" apply --reverse --check "$PATCH" >/dev/null 2>&1; then
  echo "[nemo-runtime] macos.patch is already applied"
else
  echo "[nemo-runtime] macos.patch does not apply to ${COMMIT:0:7}; check it against the new commit" >&2
  exit 1
fi

# --- SentencePiece, static ---------------------------------------------------------------------
# NeMo's own script builds SentencePiece 0.2.x from source with its bundled Abseil, protobuf-lite
# and Darts-clone, which is what we want: the archive goes inside our dylib, so the runtime folder
# needs no libsentencepiece.dylib (upstream's macOS CI installs the Homebrew formula instead).
# The script asks for a CMake policy of 3.5, because SentencePiece declares
# cmake_minimum_required(3.1) and CMake 4 refuses that without it.
if [ ! -f "$SOURCE/.deps/sentencepiece/lib/libsentencepiece.a" ]; then
  CMAKE_POLICY_VERSION_MINIMUM=3.5 JOBS="$JOBS" PREFIX="$SOURCE/.deps/sentencepiece" \
    bash "$SOURCE/scripts/build_sentencepiece_static.sh"
fi
[ -f "$SOURCE/.deps/sentencepiece/lib/libsentencepiece.a" ] || {
  echo "[nemo-runtime] the SentencePiece build produced no libsentencepiece.a" >&2; exit 1; }
SP_LICENSES=$SOURCE/.deps/sentencepiece/share/licenses/nemo-speech/third_party/sentencepiece

# --- configure and build -----------------------------------------------------------------------
# scripts/configure.sh picks the cpu-asr preset (ASR and diarization, no TTS or NMT) and its
# checks, then runs `cmake --preset cpu-asr` with these flags.
set -- "$SOURCE/scripts/configure.sh" cpu-asr
i=0
while [ $i -lt ${#FLAG_NAMES[@]} ]; do
  set -- "$@" "-D${FLAG_NAMES[$i]}=${FLAG_VALUES[$i]}"
  i=$((i + 1))
done
set -- "$@" \
  -DCMAKE_OSX_DEPLOYMENT_TARGET="$DEPLOYMENT_TARGET" \
  -DSENTENCEPIECE_STATIC_LIB="$SOURCE/.deps/sentencepiece/lib/libsentencepiece.a" \
  -DSENTENCEPIECE_INCLUDE_DIR="$SOURCE/.deps/sentencepiece/include"
CMAKE_POLICY_VERSION_MINIMUM=3.5 "$@"

# The preset configures and builds in <source>/build/cpu-asr.
PRESET_BUILD=$SOURCE/build/cpu-asr
CACHE=$PRESET_BUILD/CMakeCache.txt

# As build.ps1 reads its flags back: a flag the manifest pins but the cache doesn't hold fails
# here, so a build that isn't portable never reaches a release.
i=0
while [ $i -lt ${#FLAG_NAMES[@]} ]; do
  name=${FLAG_NAMES[$i]}
  line=$(grep -m1 "^$name:[A-Z]*=" "$CACHE" 2>/dev/null || true)
  actual=${line#*=}
  [ "$actual" = "${FLAG_VALUES[$i]}" ] || {
    echo "[nemo-runtime] the runtime was not built as pinned ($name is '${actual:-unset}', not ${FLAG_VALUES[$i]}). Delete $PRESET_BUILD and build again." >&2
    exit 1; }
  i=$((i + 1))
done
line=$(grep -m1 '^CMAKE_OSX_DEPLOYMENT_TARGET:[A-Z]*=' "$CACHE" 2>/dev/null || true)
[ "${line#*=}" = "$DEPLOYMENT_TARGET" ] || {
  echo "[nemo-runtime] the runtime would take another minimum macOS version ('${line#*=}' instead of $DEPLOYMENT_TARGET)" >&2; exit 1; }

cmake --build "$PRESET_BUILD" --parallel "$JOBS"

# --- stage the runtime folder ------------------------------------------------------------------
# Flat, versionless names with @loader_path install names, so the folder loads from wherever it is
# put: libnemo_speech_asr_c.dylib finds its five siblings by itself.
BIN=$PRESET_BUILD/bin
staging=$OUT.staging
rm -rf "$staging"
mkdir -p "$staging/licenses/sentencepiece"

# The build tree's file names carry the full version (libggml.0.12.0.dylib); SOVERSION makes the
# install names shorter (libggml.0.dylib). Copy the real file to a flat name, then rewrite what it
# records about itself.
resolve() { # $1 = library stem, e.g. libggml-base
  local match
  match=$(ls "$BIN/$1".*.dylib 2>/dev/null | sort -V | tail -1 || true)
  [ -n "$match" ] || match=$BIN/$1.dylib
  [ -f "$match" ] || { echo "[nemo-runtime] the build made no $1 dylib in $BIN" >&2; exit 1; }
  printf '%s' "$match"
}
flat_library() {
  local stem=$1 flat=$2 from
  from=$(resolve "$stem")
  cp "$from" "$staging/$flat"
  install_name_tool -id "@loader_path/$flat" "$staging/$flat"
}
flat_library libnemo_speech_asr_c libnemo_speech_asr_c.dylib
flat_library libnemo_speech_asr libnemo_speech_asr.dylib
flat_library libggml libggml.dylib
flat_library libggml-base libggml-base.dylib
flat_library libggml-cpu libggml-cpu.dylib
flat_library libggml-blas libggml-blas.dylib

for f in "$staging"/*.dylib; do
  for dep in $(otool -L "$f" | tail -n +2 | awk '{print $1}' | grep '^@rpath/' || true); do
    # libggml-base.0.dylib and libggml.0.12.0.dylib both become libggml-base.dylib and
    # libggml.dylib, the names the folder holds.
    flat=$(printf '%s' "$dep" | sed -E 's#^@rpath/##; s/(\.[0-9]+)+\.dylib$/.dylib/')
    [ -f "$staging/$flat" ] || {
      echo "[nemo-runtime] $(basename "$f") imports $dep, which the runtime folder does not hold" >&2; exit 1; }
    install_name_tool -change "$dep" "@loader_path/$flat" "$f"
  done
  # The build tree's rpath is an absolute path inside the checkout; nothing needs it now.
  for rpath in $(otool -l "$f" | awk '/LC_RPATH/{f=1} f&&/path /{print $2; f=0}' | grep -v '^@loader_path' || true); do
    install_name_tool -delete_rpath "$rpath" "$f" 2>/dev/null || true
  done
done

# Every import must be the C++ runtime, libSystem, Accelerate or another dylib of the folder, and
# every dylib has to run on macOS 15 on an Apple Silicon Mac.
for f in "$staging"/*.dylib; do
  case "$(file "$f")" in
    *arm64*) ;;
    *) echo "[nemo-runtime] $(basename "$f") is not arm64" >&2; exit 1 ;;
  esac
  minos=$(otool -l "$f" | awk '/LC_BUILD_VERSION/{b=1} b&&/minos/{print $2; exit}')
  [ "$minos" = "$DEPLOYMENT_TARGET" ] || {
    echo "[nemo-runtime] $(basename "$f") has minos ${minos:-none} instead of $DEPLOYMENT_TARGET" >&2; exit 1; }
  for dep in $(otool -L "$f" | tail -n +2 | awk '{print $1}'); do
    case "$dep" in
      @loader_path/*) ;;
      /usr/lib/libSystem.B.dylib|/usr/lib/libc++.1.dylib|/System/Library/Frameworks/Accelerate.framework/*) ;;
      *) echo "[nemo-runtime] $(basename "$f") imports $dep, which the runtime folder cannot satisfy" >&2; exit 1 ;;
    esac
  done
done

cp "$SOURCE/LICENSE" "$SOURCE/NOTICE" "$SOURCE/THIRD_PARTY_NOTICES.md" "$staging/licenses/"
for name in LICENSE absl-LICENSE darts-clone-LICENSE protobuf-lite-LICENSE; do
  [ -f "$SP_LICENSES/$name" ] || { echo "[nemo-runtime] SentencePiece's $name is missing" >&2; exit 1; }
  cp "$SP_LICENSES/$name" "$staging/licenses/sentencepiece/$name"
done

# runtime.json: the pins, the flags, the toolset and the size and SHA-256 of every file, in the
# shape build.ps1 writes, with the target this folder belongs to.
python3 - "$staging" "$MANIFEST" "$TARGET" "$RUNNER" <<'PY'
import hashlib, json, pathlib, subprocess, sys
staging, manifest_path, target, runner = sys.argv[1:5]
manifest = json.load(open(manifest_path))
entry = manifest["targets"][target]
root = pathlib.Path(staging)
files = []
for path in sorted(root.rglob("*")):
    if not path.is_file() or path.name == "runtime.json":
        continue
    data = path.read_bytes()
    files.append({
        "path": path.relative_to(root).as_posix(),
        "size": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
    })
def version(*args):
    try:
        output = subprocess.run(args, capture_output=True, text=True).stdout
        return output.strip().splitlines()[0] if output.strip() else "unknown"
    except Exception:
        return "unknown"
doc = {
    "format": 1,
    "target": target,
    "name": manifest["name"],
    "repository": manifest["repository"],
    "commit": manifest["commit"],
    "submodules": manifest["submodules"],
    "build": entry["build"],
    "flags": entry["flags"],
    "toolset": {
        "runner": runner,
        "xcode": version("xcodebuild", "-version"),
        "clang": version("clang", "--version"),
        "cmake": version("cmake", "--version"),
        "deploymentTarget": entry["toolset"]["deploymentTarget"],
    },
    "cpu": entry["cpu"],
    "files": files,
}
(root / "runtime.json").write_text(json.dumps(doc, indent=2) + "\n")
print(f"[nemo-runtime] runtime.json: {len(files)} files")
PY

rm -rf "$OUT"
mv "$staging" "$OUT"
echo "[nemo-runtime] runtime staged in $OUT"
for f in "$OUT"/*.dylib; do printf '  %10d  %s\n' "$(stat -f %z "$f")" "$(basename "$f")"; done
