//! Keeps the audio of a dictation that inserted nothing, so a miss can be looked into later.
//!
//! A dictation that ends with "nothing heard" or an empty transcript is indistinguishable from a
//! microphone that delivered no usable sound, and the outcome line can't carry the audio. This
//! writes that audio to a WAV file in the data folder instead.
//!
//! Nothing here runs unless Voice's options say so: a development copy started with
//! `RUKOO_VOICE_DUMP_EMPTY_DICTATION=1`, in a build that keeps debug assertions. A release build
//! never asks for it.
//!
//! # Privacy
//!
//! The file holds exactly what the person said during that hold, uncut and unencrypted, next to
//! the rest of the data folder. It is never uploaded, never read back by Voice, and nothing
//! deletes it. Treat it like a recording of the room: it may hold private words said out loud,
//! so keep it only until the miss is explained and delete the folder afterwards. A normal run
//! writes nothing here.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use voice_audio::segment_header;

/// The folder under the data folder these files go in.
pub const FOLDER: &str = "dictation-dumps";

/// Writes one dictation's audio as a 16 kHz mono WAV and returns where it went. Fails only when
/// the folder or the file can't be written, which the caller reports and then carries on. A name
/// that is already taken gets a number, so an earlier miss is never overwritten.
pub fn save(data_dir: &Path, id: u64, pcm: &[i16]) -> io::Result<PathBuf> {
    let folder = data_dir.join(FOLDER);
    fs::create_dir_all(&folder)?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0);
    let mut bytes = Vec::with_capacity(segment_header(pcm.len() as u64).len() + pcm.len() * 2);
    bytes.extend_from_slice(&segment_header(pcm.len() as u64));
    for sample in pcm {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    for copy in 1..=20 {
        let suffix = if copy == 1 {
            String::new()
        } else {
            format!("-{copy}")
        };
        let path = folder.join(format!("dictation-{id}-{millis}{suffix}.wav"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(&bytes)?;
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("every name for this dictation was taken"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rukoo-voice-dump-{name}-{}-{:?}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saves_the_audio_as_a_wav_in_the_data_folder() {
        let data = temp_dir("save");
        let pcm: Vec<i16> = (0..32_000).map(|i| (i % 200) as i16 - 100).collect();
        let path = save(&data, 7, &pcm).unwrap();

        assert_eq!(path.parent().unwrap(), data.join(FOLDER));
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("dictation-7-"), "{name}");
        assert!(name.ends_with(".wav"), "{name}");
        assert_eq!(fs::read(&path).unwrap().len(), 44 + pcm.len() * 2);
        let (read, rate) = voice_audio::read_wav_i16(&path).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(read, pcm);
        fs::remove_dir_all(data).unwrap();
    }

    /// Two misses of the same recording never overwrite each other.
    #[test]
    fn every_save_gets_its_own_file() {
        let data = temp_dir("twice");
        let first = save(&data, 3, &[1, 2, 3]).unwrap();
        let second = save(&data, 3, &[4, 5, 6]).unwrap();
        assert_ne!(first, second);
        assert_eq!(voice_audio::read_wav_i16(&first).unwrap().0, [1, 2, 3]);
        assert_eq!(voice_audio::read_wav_i16(&second).unwrap().0, [4, 5, 6]);
        fs::remove_dir_all(data).unwrap();
    }
}
