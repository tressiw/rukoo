//! `rukoo-voice-screen inspect <file.mp4>`: what a video file holds, from the box walker on every
//! platform and, on Windows, from Media Foundation's source reader too, which reads the samples
//! the way a player would. macOS reads the samples through the box walker (`voice_screen_files`)
//! as well, so the report has the same shape without Media Foundation.

use std::path::Path;

use serde_json::{json, Value};
use voice_screen_files::mp4;

/// `{ bytes, complete, completeBytes, timescale, fragments, fragmentMs, durationMs, minFragmentMs,
/// maxFragmentMs, samples, syncFragments, keyframes, reader }`, or `{ error }`. `reader` is
/// `{ samples, keyframes, lastMs, width, height }`: from Media Foundation on Windows, from the
/// box walker on macOS, `{ error }` when it can't read the file, and `null` on Linux.
pub fn inspect(path: &Path) -> Value {
    let index = match mp4::index_path(path) {
        Ok(index) => index,
        Err(e) => return json!({ "error": format!("can't read {}: {e}", path.display()) }),
    };
    let fragment_ms = mp4::fragment_ms(&index);
    let samples: u64 = index.fragments.iter().map(|f| u64::from(f.samples)).sum();
    let sync_fragments = index.fragments.iter().filter(|f| f.first_sync).count();
    let duration_ms = match index.fragments.last() {
        Some(last) if index.timescale > 0 => index.to_ms(last.end()),
        _ => 0,
    };
    let reader = read_samples(path);
    let keyframes = reader
        .get("keyframes")
        .and_then(Value::as_u64)
        .unwrap_or(sync_fragments as u64);
    json!({
        "bytes": index.file_len,
        "complete": index.complete_len == index.file_len,
        "completeBytes": index.complete_len,
        "timescale": index.timescale,
        "fragments": index.fragments.len(),
        "fragmentMs": fragment_ms,
        "durationMs": duration_ms,
        "minFragmentMs": fragment_ms.iter().min(),
        "maxFragmentMs": fragment_ms.iter().max(),
        "samples": samples,
        "syncFragments": sync_fragments,
        "keyframes": keyframes,
        "reader": reader,
    })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn read_samples(_path: &Path) -> Value {
    Value::Null
}

/// What the shared MP4 box walker reads of the file: its samples, the fragments that start on a
/// keyframe, where the media ends, and the size the `avc1` sample entry declares.
#[cfg(target_os = "macos")]
fn read_samples(path: &Path) -> Value {
    let Ok(index) = mp4::index_path(path) else {
        return json!({ "error": format!("can't read {}", path.display()) });
    };
    let samples: u64 = index.fragments.iter().map(|f| u64::from(f.samples)).sum();
    let keyframes = index.fragments.iter().filter(|f| f.first_sync).count() as u64;
    let last_ms = index
        .fragments
        .last()
        .map_or(0, |fragment| index.to_ms(fragment.end()));
    let (width, height) = file_size(path, &index).unwrap_or((0, 0));
    json!({
        "samples": samples,
        "keyframes": keyframes,
        "lastMs": last_ms,
        "width": width,
        "height": height,
        "error": Value::Null,
    })
}

/// The picture size from the file's `avc1` sample entry, which the encoder wrote.
#[cfg(target_os = "macos")]
fn file_size(path: &Path, index: &mp4::Index) -> Option<(u32, u32)> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).ok()?;
    let mut moov = vec![0u8; (index.moov.end - index.moov.start) as usize];
    file.seek(SeekFrom::Start(index.moov.start)).ok()?;
    file.read_exact(&mut moov).ok()?;
    // The entry is a box: four bytes of size, then `avc1`, then 24 bytes before the width and
    // height, as `voice_screen_files::fmp4` writes them.
    let at = moov.windows(8).position(|window| &window[4..] == b"avc1")?;
    let payload = at + 8;
    let width = u32::from(u16::from_be_bytes(
        moov.get(payload + 24..payload + 26)?.try_into().ok()?,
    ));
    let height = u32::from(u16::from_be_bytes(
        moov.get(payload + 26..payload + 28)?.try_into().ok()?,
    ));
    Some((width, height))
}

#[cfg(windows)]
fn read_samples(path: &Path) -> Value {
    use windows::core::HSTRING;
    use windows::Win32::Media::MediaFoundation::*;

    /// `MF_SOURCE_READER_FIRST_VIDEO_STREAM`.
    const FIRST_VIDEO: u32 = 0xFFFF_FFFC;

    crate::com::init_mta();
    let Some(mf) = crate::video::mf::shared() else {
        return json!({ "error": "Media Foundation isn't available on this computer" });
    };
    let url = HSTRING::from(path.as_os_str());
    let reader = match mf.create_source_reader(&url) {
        Ok(reader) => reader,
        Err(e) => return json!({ "error": format!("{:#010x} {}", e.code().0, e.message()) }),
    };
    let (width, height) = unsafe { reader.GetCurrentMediaType(FIRST_VIDEO) }
        .and_then(|t| unsafe { t.GetUINT64(&MF_MT_FRAME_SIZE) })
        .map(|size| ((size >> 32) as u32, size as u32))
        .unwrap_or((0, 0));
    let (mut samples, mut keyframes, mut last) = (0u64, 0u64, 0i64);
    let mut error = Value::Null;
    loop {
        let mut flags = 0u32;
        let mut time = 0i64;
        let mut sample: Option<IMFSample> = None;
        let read = unsafe {
            reader.ReadSample(
                FIRST_VIDEO,
                0,
                None,
                Some(&mut flags),
                Some(&mut time),
                Some(&mut sample),
            )
        };
        if let Err(e) = read {
            error = json!(format!("{:#010x} {}", e.code().0, e.message()));
            break;
        }
        if flags & MF_SOURCE_READERF_ERROR.0 as u32 != 0 {
            error = json!("the stream reported an error");
            break;
        }
        if let Some(sample) = sample {
            samples += 1;
            last = last.max(time);
            if unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint) }.unwrap_or(0) != 0 {
                keyframes += 1;
            }
        }
        if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
            break;
        }
    }
    json!({
        "samples": samples,
        "keyframes": keyframes,
        "lastMs": last / 10_000,
        "width": width,
        "height": height,
        "error": error,
    })
}
