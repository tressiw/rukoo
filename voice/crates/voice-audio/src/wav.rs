//! WAV segments: 16 kHz, mono, signed 16-bit little-endian PCM with the canonical 44-byte header.
//!
//! The writer keeps the header honest after every append: it writes the samples first and then
//! patches the RIFF and data sizes. A hard kill between the two leaves a header that counts
//! fewer samples than the file holds, never more, so the file always opens.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::SAMPLE_RATE;

/// Length of the canonical header; the samples start right after it.
pub const HEADER_LEN: u64 = 44;
/// The most samples one segment can hold: the RIFF size field is 32 bits.
pub const MAX_SEGMENT_SAMPLES: u64 = (u32::MAX as u64 - 36) / 2;

/// The canonical header for a segment holding `samples` samples.
pub fn segment_header(samples: u64) -> [u8; 44] {
    let data = (samples.min(MAX_SEGMENT_SAMPLES) * 2) as u32;
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    h[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    h[28..32].copy_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    h[32..34].copy_from_slice(&2u16.to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

/// Writes one segment file. Not thread-safe on its own; one owner appends.
pub struct SegmentWriter {
    file: File,
    path: PathBuf,
    samples: u64,
}

impl SegmentWriter {
    /// Creates a new segment. Fails if the file already exists, so a resumed recording can never
    /// overwrite audio from an earlier session. Takes the one writer's lock, so a second recorder
    /// can't write into a live segment.
    pub fn create(path: &Path) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        exclusive_writer(&mut options);
        let file = options.open(path)?;
        lock_writer(&file)?;
        write_all_at(&file, &segment_header(0), 0)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            samples: 0,
        })
    }

    /// Appends samples and then updates the header to count them.
    pub fn append(&mut self, pcm: &[i16]) -> io::Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        let total = self.samples + pcm.len() as u64;
        if total > MAX_SEGMENT_SAMPLES {
            return Err(io::Error::other("segment exceeds the WAV size limit"));
        }
        let mut bytes = Vec::with_capacity(pcm.len() * 2);
        for sample in pcm {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        write_all_at(&self.file, &bytes, HEADER_LEN + self.samples * 2)?;
        // One write covers both size fields (offsets 4 and 40); the bytes between them don't
        // change.
        write_all_at(&self.file, &segment_header(total)[4..], 4)?;
        self.samples = total;
        Ok(())
    }

    /// Asks the OS to put the samples written so far on the disk.
    pub fn sync(&self) -> io::Result<()> {
        self.file.sync_data()
    }

    /// Syncs and closes the segment. Returns the number of samples in it.
    pub fn close(self) -> io::Result<u64> {
        self.file.sync_data()?;
        Ok(self.samples)
    }

    pub fn samples(&self) -> u64 {
        self.samples
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Audio read from a WAV file, converted to signed 16-bit.
#[derive(Debug, Clone, PartialEq)]
pub struct Wav {
    pub rate: u32,
    pub channels: u16,
    /// Interleaved frames.
    pub samples: Vec<i16>,
}

/// Reads a WAV file as mono 16-bit samples and its rate. Several channels are averaged.
///
/// A data size that counts fewer bytes than the file holds wins: the rest is an append the
/// writer hadn't counted yet. A data size of 0 or `0xFFFFFFFF` means unknown, and the file
/// length is used, rounded down to whole samples.
pub fn read_wav_i16(path: &Path) -> io::Result<(Vec<i16>, u32)> {
    let wav = read_wav(path)?;
    if wav.channels == 1 {
        return Ok((wav.samples, wav.rate));
    }
    let channels = wav.channels as usize;
    let mono = wav
        .samples
        .chunks_exact(channels)
        .map(|frame| {
            let sum: i32 = frame.iter().map(|&s| s as i32).sum();
            (sum / channels as i32) as i16
        })
        .collect();
    Ok((mono, wav.rate))
}

/// Reads PCM (8, 16, 24 or 32-bit) or 32-bit float WAV files, including
/// `WAVE_FORMAT_EXTENSIBLE`, with the same size tolerance as [`read_wav_i16`].
pub fn read_wav(path: &Path) -> io::Result<Wav> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    parse_wav(&bytes)
}

#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    Int,
    Float,
}

struct Format {
    encoding: Encoding,
    channels: u16,
    rate: u32,
    bits: u16,
    block_align: u16,
}

fn parse_wav(bytes: &[u8]) -> io::Result<Wav> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(invalid("not a WAV file"));
    }
    // The RIFF size is ignored: a crashed writer may have left it at 0.
    let mut at = 12usize;
    let mut format: Option<Format> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
        let body = at + 8;
        if id == b"data" {
            let format = format.ok_or_else(|| invalid("WAV data before its format"))?;
            let available = (bytes.len() - body) as u64;
            let declared = size as u64;
            let len = if declared == 0 || declared == u32::MAX as u64 {
                available
            } else {
                declared.min(available)
            };
            let len = len - len % format.block_align as u64;
            return Ok(decode(&bytes[body..body + len as usize], &format));
        }
        let end = body
            .checked_add(size as usize)
            .filter(|&end| end <= bytes.len())
            .ok_or_else(|| invalid("WAV chunk runs past the end of the file"))?;
        if id == b"fmt " {
            format = Some(parse_format(&bytes[body..end])?);
        }
        // Chunks are padded to an even length.
        at = end + (size as usize & 1);
    }
    Err(invalid("WAV file has no data"))
}

fn parse_format(fmt: &[u8]) -> io::Result<Format> {
    if fmt.len() < 16 {
        return Err(invalid("WAV format chunk is too short"));
    }
    let u16_at = |i: usize| u16::from_le_bytes([fmt[i], fmt[i + 1]]);
    let mut tag = u16_at(0);
    let channels = u16_at(2);
    let rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
    let block_align = u16_at(12);
    let bits = u16_at(14);
    if tag == 0xFFFE {
        // WAVE_FORMAT_EXTENSIBLE: the sub-format GUID starts with the plain format tag.
        if fmt.len() < 26 {
            return Err(invalid("WAV extensible format chunk is too short"));
        }
        tag = u16_at(24);
    }
    let encoding = match (tag, bits) {
        (1, 8 | 16 | 24 | 32) => Encoding::Int,
        (3, 32) => Encoding::Float,
        _ => {
            return Err(invalid(&format!(
                "unsupported WAV encoding (format {tag}, {bits} bits)"
            )))
        }
    };
    if channels == 0 || rate == 0 || block_align as u32 != channels as u32 * (bits as u32 / 8) {
        return Err(invalid("WAV format chunk is inconsistent"));
    }
    Ok(Format {
        encoding,
        channels,
        rate,
        bits,
        block_align,
    })
}

fn decode(data: &[u8], format: &Format) -> Wav {
    let width = format.bits as usize / 8;
    let samples = data
        .chunks_exact(width)
        .map(|b| match (format.encoding, width) {
            (Encoding::Int, 1) => ((b[0] as i16) - 128) << 8,
            (Encoding::Int, 2) => i16::from_le_bytes([b[0], b[1]]),
            (Encoding::Int, 3) => i16::from_le_bytes([b[1], b[2]]),
            (Encoding::Int, _) => i16::from_le_bytes([b[2], b[3]]),
            (Encoding::Float, _) => crate::f32_to_i16(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        })
        .collect();
    Wav {
        rate: format.rate,
        channels: format.channels,
        samples,
    }
}

/// Fixes a segment's header from the file's length and returns the number of samples.
///
/// Recovers the samples of a last append the writer didn't count before it died, drops a
/// trailing odd byte, and gives a file cut off inside its header a fresh empty header. Refuses
/// files that aren't segments in the stored format, so it never rewrites someone else's WAV.
///
/// It is a writer, so it takes the same lock as [`SegmentWriter`]: while a recorder writes a
/// segment, its repair fails with [`io::ErrorKind::WouldBlock`] rather than fighting over it.
pub fn repair_segment(path: &Path) -> io::Result<u64> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    lock_writer(&file)?;
    let len = file.metadata()?.len();
    let mut head = Vec::with_capacity(HEADER_LEN as usize);
    (&mut file).take(HEADER_LEN).read_to_end(&mut head)?;
    if !is_segment_header(&head) {
        return Err(invalid("not a Rukoo Voice audio segment"));
    }
    let samples = (len.saturating_sub(HEADER_LEN) / 2).min(MAX_SEGMENT_SAMPLES);
    let end = HEADER_LEN + samples * 2;
    if len > end {
        file.set_len(end)?;
    }
    let header = segment_header(samples);
    if head[..] != header[..] {
        write_all_at(&file, &header, 0)?;
    }
    file.sync_data()?;
    Ok(samples)
}

/// True when `head` is our header, or the start of it, whatever its two size fields say.
fn is_segment_header(head: &[u8]) -> bool {
    let canonical = segment_header(0);
    head.iter()
        .zip(canonical.iter())
        .enumerate()
        .all(|(i, (a, b))| (4..8).contains(&i) || (40..44).contains(&i) || a == b)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// Other processes may read a segment while it's written, but not open it for writing.
#[cfg(windows)]
fn exclusive_writer(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_DELETE: u32 = 0x4;
    options.share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE);
}

#[cfg(not(windows))]
fn exclusive_writer(_options: &mut OpenOptions) {}

/// Keeps a second writer off a segment. Windows refused it at open through the share mode;
/// elsewhere an exclusive `flock` on the open file does it. It belongs to the file, so a hard kill
/// releases it, and readers never take it.
///
/// `WouldBlock`, what EWOULDBLOCK reads as, means another writer holds the segment.
#[cfg(unix)]
fn lock_writer(file: &File) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    loop {
        // SAFETY: `flock` only reads the descriptor; the lock belongs to the file, not to memory.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        // A signal in the call changes nothing, so try again.
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(not(unix))]
fn lock_writer(_file: &File) -> io::Result<()> {
    Ok(())
}

/// Positional writes, so the header patch and the next append never depend on a file cursor.
#[cfg(unix)]
fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, offset)
}

#[cfg(windows)]
fn write_all_at(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        match file.seek_write(buf, offset) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                buf = &buf[n..];
                offset += n as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir()
                .join(format!("voice-audio-{name}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ramp(n: usize) -> Vec<i16> {
        (0..n).map(|i| (i as i16).wrapping_mul(7)).collect()
    }

    fn header_sizes(path: &Path) -> (u32, u32) {
        let bytes = std::fs::read(path).unwrap();
        let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let data = u32::from_le_bytes(bytes[40..44].try_into().unwrap());
        (riff, data)
    }

    #[test]
    fn header_is_canonical_and_hound_reads_it() {
        let dir = TempDir::new("header");
        let path = dir.file("000001.wav");
        let mut writer = SegmentWriter::create(&path).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 44);
        assert_eq!(header_sizes(&path), (36, 0));
        writer.append(&ramp(1600)).unwrap();
        writer.append(&ramp(320)).unwrap();
        assert_eq!(header_sizes(&path), (36 + 3840, 3840));
        assert_eq!(writer.close().unwrap(), 1920);

        let reader = hound::WavReader::open(&path).unwrap();
        let spec = reader.spec();
        assert_eq!(
            (spec.channels, spec.sample_rate, spec.bits_per_sample),
            (1, 16_000, 16)
        );
        let samples: Vec<i16> = reader.into_samples().map(Result::unwrap).collect();
        let mut expected = ramp(1600);
        expected.extend(ramp(320));
        assert_eq!(samples, expected);
    }

    #[test]
    fn never_overwrites_an_existing_segment() {
        let dir = TempDir::new("exists");
        let path = dir.file("000001.wav");
        SegmentWriter::create(&path).unwrap();
        let err = SegmentWriter::create(&path).err().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    /// Windows reports its share mode as a sharing violation and Unix as a lock it can't take;
    /// both stop the second writer before it changes a byte. The lock of a killed recorder is in
    /// `tests/lock.rs`, which needs a process of its own.
    #[cfg(unix)]
    #[test]
    fn a_locked_segment_refuses_a_second_writer_but_opens_for_readers() {
        let dir = TempDir::new("lock");
        let path = dir.file("000001.wav");
        let mut writer = SegmentWriter::create(&path).unwrap();
        writer.append(&ramp(100)).unwrap();
        let before = std::fs::read(&path).unwrap();
        let err = repair_segment(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock, "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // A reader takes no lock, so a live segment still opens: live transcription reads it and
        // `finalize` prepares for the next session from a closed one.
        assert_eq!(read_wav_i16(&path).unwrap().0, ramp(100));
        drop(writer);
        // The writer closed the file, so the next one gets the lock.
        assert_eq!(repair_segment(&path).unwrap(), 100);
    }

    #[test]
    fn reader_trusts_a_header_that_undercounts() {
        // A kill after the samples were written but before the sizes were patched.
        let dir = TempDir::new("undercount");
        let path = dir.file("000001.wav");
        let mut writer = SegmentWriter::create(&path).unwrap();
        writer.append(&ramp(100)).unwrap();
        drop(writer);
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&[1, 0, 2, 0, 3]).unwrap();
        drop(file);
        let (samples, rate) = read_wav_i16(&path).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(samples, ramp(100));
        assert!(hound::WavReader::open(&path).is_ok());
    }

    #[test]
    fn reader_uses_the_file_length_when_the_sizes_are_zero() {
        let dir = TempDir::new("zero");
        let path = dir.file("000001.wav");
        let mut header = segment_header(0);
        header[4..8].copy_from_slice(&0u32.to_le_bytes());
        let mut bytes = header.to_vec();
        for s in ramp(50) {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        bytes.push(9); // half a sample
        std::fs::write(&path, &bytes).unwrap();
        let (samples, _) = read_wav_i16(&path).unwrap();
        assert_eq!(samples, ramp(50));
    }

    #[test]
    fn repair_counts_every_whole_sample_on_disk() {
        let dir = TempDir::new("repair");
        let path = dir.file("000001.wav");
        let mut writer = SegmentWriter::create(&path).unwrap();
        writer.append(&ramp(10)).unwrap();
        drop(writer);
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&[5, 0, 6, 0, 7]).unwrap();
        drop(file);

        assert_eq!(repair_segment(&path).unwrap(), 12);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 44 + 24);
        assert_eq!(header_sizes(&path), (36 + 24, 24));
        let (samples, _) = read_wav_i16(&path).unwrap();
        let mut expected = ramp(10);
        expected.extend([5, 6]);
        assert_eq!(samples, expected);
        // Running it again changes nothing.
        let before = std::fs::read(&path).unwrap();
        assert_eq!(repair_segment(&path).unwrap(), 12);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn repair_gives_a_cut_off_header_a_fresh_one() {
        let dir = TempDir::new("short");
        let path = dir.file("000001.wav");
        std::fs::write(&path, &segment_header(0)[..20]).unwrap();
        assert_eq!(repair_segment(&path).unwrap(), 0);
        assert_eq!(std::fs::read(&path).unwrap(), segment_header(0));
        std::fs::write(&path, b"").unwrap();
        assert_eq!(repair_segment(&path).unwrap(), 0);
        assert!(hound::WavReader::open(&path).is_ok());
    }

    #[test]
    fn repair_refuses_other_wav_files() {
        let dir = TempDir::new("foreign");
        let path = dir.file("music.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for s in 0..100i16 {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
        let before = std::fs::read(&path).unwrap();
        assert_eq!(
            repair_segment(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn reads_other_formats_for_file_sources() {
        let dir = TempDir::new("formats");
        let stereo = dir.file("stereo.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 22_050,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&stereo, spec).unwrap();
        for (l, r) in [(100i16, 300i16), (-200, -400)] {
            w.write_sample(l).unwrap();
            w.write_sample(r).unwrap();
        }
        w.finalize().unwrap();
        let wav = read_wav(&stereo).unwrap();
        assert_eq!((wav.rate, wav.channels), (22_050, 2));
        assert_eq!(wav.samples, vec![100, 300, -200, -400]);
        assert_eq!(read_wav_i16(&stereo).unwrap(), (vec![200, -300], 22_050));

        let float = dir.file("float.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::create(&float, spec).unwrap();
        for v in [0.5f32, -1.0, 2.0] {
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let wav = read_wav(&float).unwrap();
        assert_eq!(wav.rate, 48_000);
        assert_eq!(wav.samples, vec![16_384, -32_767, 32_767]);
    }

    #[test]
    fn skips_chunks_before_the_data() {
        let mut bytes = b"RIFF\0\0\0\0WAVE".to_vec();
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&segment_header(0)[16..36]);
        bytes.extend_from_slice(b"LIST");
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(b"abc\0"); // odd size, padded
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 0, 2, 0]);
        let wav = parse_wav(&bytes).unwrap();
        assert_eq!(wav.samples, vec![1, 2]);
    }

    #[test]
    fn rejects_broken_files() {
        assert!(parse_wav(b"RIFF\0\0\0\0WAVEdata\0\0\0\0").is_err());
        assert!(parse_wav(b"not a wav").is_err());
        let mut bytes = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        bytes.extend_from_slice(&100u32.to_le_bytes());
        assert!(parse_wav(&bytes).is_err());
    }
}
