//! The fragmented MP4 writer the macOS screen process feeds VideoToolbox's H.264 samples to.
//!
//! A platform-neutral writer, so its tests run on every platform the crate builds on: the caller
//! brings the encoder's parameter sets and its AVCC samples (4-byte big-endian NAL lengths, which
//! is what an `avcC` box declares), and this module writes the file. The layout is the one
//! `voice/RECORDING.md` and [`crate::mp4`] describe:
//!
//! ```text
//! ftyp, moov (one avc1 track with avcC, and mvex/trex),
//! moof, mdat, moof, mdat, ...,
//! mfra (tfra)
//! ```
//!
//! What the reader and the cutter need, and what the writer keeps:
//!
//! - `tfhd` carries `default-base-is-moof`, so `trun`'s data offset is relative to the `moof`
//!   and [`crate::mp4::cut`] can copy a fragment without rewriting anything but its numbers.
//! - `tfdt` version 1 states each fragment's base decode time, and media time 0 is the first
//!   sample. The caller hands over meeting-clock times; the writer rebases them on the first
//!   sample and quantises them to the track's timescale ([`TIMESCALE`], 1/15,000 s, as
//!   RECORDING.md requires).
//! - One `trun` per fragment with each sample's duration, size and sync flag. A sample lasts
//!   until the next sample's time, so a pause on the screen becomes one long sample and the
//!   file's duration is `video_end.t - video.t` exactly. The newest sample waits in the writer
//!   until the next one or [`Mp4Writer::finish`] gives it its duration; the last sample gets one
//!   nominal frame.
//! - A fragment ends at the first sync sample once the open one holds [`MIN_FRAGMENT`] (1 s), so
//!   fragments are 1 to 2 s and each starts on a sync sample. [`Mp4Writer::needs_keyframe`]
//!   tells the caller to encode the next sample as a keyframe.
//! - Each `moof`+`mdat` pair reaches the file in one `write`, so a hard kill tears at most the
//!   last pair. `mp4::index_path` then sees every complete fragment and `mp4::repair` cuts the
//!   torn tail, which leaves a file that plays to its last complete fragment.
//!
//! The caller starts with a sync sample, as every encoder's first frame is. Fragments are not
//! synced one by one: each pair is written through the page cache, so a hard kill keeps
//! everything already written, and the writer asks the OS to put the file on the disk at most
//! every 5 s, like the recorder's WAV segments. Syncing every 1–2 s fragment would flush the
//! disk once or twice a second for the whole meeting to gain at most a couple of seconds against
//! a power cut; the 5 s rule keeps that loss near the WAV path's and [`Mp4Writer::finish`] syncs
//! everything before the file is announced with `video_end`.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::mp4::{bx, full};

/// The track timescale of the screen's video files: 15,000 units a second, as in RECORDING.md.
pub const TIMESCALE: u32 = 15_000;
/// The nominal frame rate of the screen's video, in the `video` line's `fps`.
pub const FPS: u32 = 15;
/// The shortest fragment, in timescale units: one second, as on Windows.
pub const MIN_FRAGMENT: u64 = TIMESCALE as u64;
/// `trun` sample flags: a sync sample (a keyframe).
pub const SYNC_FLAGS: u32 = 0x0200_0000;
/// `trun` sample flags: depends on others and isn't a sync sample.
pub const NON_SYNC_FLAGS: u32 = 0x0101_0000;

/// How long the writer may go between asking the OS to put fragments on the disk.
const SYNC_AFTER: Duration = Duration::from_secs(5);
/// The track id of the file's only track.
const TRACK_ID: u32 = 1;

/// Why a file couldn't be written.
#[derive(Debug)]
pub enum Error {
    /// A value the format keeps in a 32-bit field doesn't fit: a sample's size or duration, a
    /// fragment's `mdat`, the sample count, or an SPS or PPS longer than its 16-bit length.
    TooLarge {
        what: &'static str,
        value: u64,
    },
    /// The writer can't make the file from these arguments.
    Invalid(&'static str),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::TooLarge { what, value } => {
                write!(f, "a {what} of {value} doesn't fit the format")
            }
            Error::Invalid(why) => f.write_str(why),
            Error::Io(e) => write!(f, "can't write the video file: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}

/// `value` as a 32-bit field, or the error that keeps a corrupt file from being written.
fn fits(value: u64, what: &'static str) -> Result<u32, Error> {
    u32::try_from(value).map_err(|_| Error::TooLarge { what, value })
}

/// One sample waiting for its duration: the newest the writer has, and the samples of the open
/// fragment, whose durations the next sample settled.
struct Sample {
    /// Media time in timescale units, counted from the first sample.
    units: u64,
    sync: bool,
    data: Vec<u8>,
}

/// A sample of the open fragment, with its duration.
struct Written {
    units: u64,
    duration: u32,
    sync: bool,
    data: Vec<u8>,
}

/// A sync sample that starts a fragment, for `mfra`.
struct SyncPoint {
    time: u64,
    moof: u64,
}

/// Writes one fragmented MP4 file; one owner appends.
pub struct Mp4Writer {
    file: File,
    path: PathBuf,
    timescale: u32,
    /// The meeting time of media time 0, from the first sample.
    origin: Option<u64>,
    /// The media time of the newest sample taken, which the next one must pass.
    newest: Option<u64>,
    /// The newest sample, waiting for the next sample's time to give it its duration.
    pending: Option<Sample>,
    /// The open fragment's samples, all with their duration.
    open: Vec<Written>,
    /// Media time of the open fragment's first sample.
    open_start: Option<u64>,
    sequence: u32,
    sync_points: Vec<SyncPoint>,
    /// When the file was last synced, for the 5 s rule.
    last_sync: Instant,
    /// The bytes written so far.
    written: u64,
    min_fragment: u64,
}

impl Mp4Writer {
    /// Creates `path`, which must not exist, and writes its `ftyp` and `moov`. The folder must be
    /// there.
    ///
    /// `sps` and `pps` are the encoder's parameter sets without start codes, in the order
    /// VideoToolbox hands them over. The `avcC` box takes its version 1 header from them: profile,
    /// compatibility and level from the SPS's first four bytes, and then both sets with their
    /// 16-bit lengths.
    pub fn create(
        path: &Path,
        timescale: u32,
        width: u16,
        height: u16,
        sps: &[u8],
        pps: &[u8],
    ) -> Result<Mp4Writer, Error> {
        if timescale == 0 {
            return Err(Error::Invalid("the timescale is 0"));
        }
        let head = [ftyp(), moov(timescale, width, height, &avcc(sps, pps)?)].concat();
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(Error::Io)?;
        if let Err(e) = file.write_all(&head) {
            // Nothing keeps a failed start at `path`, so its number can be used again.
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(Error::Io(e));
        }
        Ok(Mp4Writer {
            file,
            path: path.to_path_buf(),
            timescale,
            origin: None,
            newest: None,
            pending: None,
            open: Vec::new(),
            open_start: None,
            sequence: 0,
            sync_points: Vec::new(),
            last_sync: Instant::now(),
            written: head.len() as u64,
            min_fragment: MIN_FRAGMENT,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn timescale(&self) -> u32 {
        self.timescale
    }

    /// Sets the shortest fragment, in timescale units. One second is the rule the screen
    /// process uses; a test may want another.
    pub fn set_min_fragment(&mut self, units: u64) {
        self.min_fragment = units;
    }

    /// Adds the sample captured at meeting time `t`, in ms.
    ///
    /// The sample reaches the file when the next sample or [`Mp4Writer::finish`] gives it its
    /// duration, so the caller can record every frame at its own capture time without knowing
    /// how far the next one is. `t` must not go back: a time at or before the previous sample's
    /// becomes the previous sample's plus one unit, as the Windows writer keeps a frame captured
    /// no later than the last one after it.
    ///
    /// A size or a duration that a 32-bit field can't hold is refused before anything is
    /// written or changed, so the caller sees a corrupt file's cause instead of the file.
    pub fn add(&mut self, t: u64, sync: bool, data: &[u8]) -> Result<(), Error> {
        fits(data.len() as u64, "sample size")?;
        let units = match self.origin {
            // The first sample is media time 0, whatever its meeting time.
            None => 0,
            Some(_) => {
                let at = self.units_at(t);
                match self.newest {
                    Some(last) => at.max(last.saturating_add(1)),
                    None => at,
                }
            }
        };
        let duration = match &self.pending {
            Some(pending) => Some(fits(
                units.saturating_sub(pending.units),
                "sample duration",
            )?),
            None => None,
        };
        if self.origin.is_none() {
            self.origin = Some(t);
        }
        self.newest = Some(units);
        if let (Some(pending), Some(duration)) = (self.pending.take(), duration) {
            self.push(pending, duration)?;
        }
        self.pending = Some(Sample {
            units,
            sync,
            data: data.to_vec(),
        });
        Ok(())
    }

    /// Whether the caller should encode the sample at meeting time `t` as a keyframe: the
    /// fragment that sample would join already holds [`MIN_FRAGMENT`], so its keyframe starts the
    /// next one. A keyframe the caller already produced that will end the open fragment counts as
    /// the start of the next fragment, so the call after adding it answers false; forcing a
    /// keyframe twice does no harm.
    pub fn needs_keyframe(&self, t: u64) -> bool {
        let Some(start) = self.next_fragment_start() else {
            return false;
        };
        self.units_at(t).saturating_sub(start) >= self.min_fragment
    }

    /// The media time where the next fragment starts: the held sync sample that will end the open
    /// fragment, once its duration arrives, or the open fragment's first sample.
    fn next_fragment_start(&self) -> Option<u64> {
        match self.open_start {
            Some(start) => match &self.pending {
                Some(pending)
                    if pending.sync && pending.units.saturating_sub(start) >= self.min_fragment =>
                {
                    Some(pending.units)
                }
                _ => Some(start),
            },
            None => self.pending.as_ref().map(|pending| pending.units),
        }
    }

    /// The meeting time where the file ends so far, as `video_end` writes it: the newest sample's
    /// start plus one nominal frame, because its duration isn't known yet. [`Mp4Writer::finish`]
    /// gives that sample the same duration, so the finished file ends here. 0 before the first
    /// sample.
    pub fn end(&self) -> u64 {
        let Some(origin) = self.origin else {
            return 0;
        };
        let units = match &self.pending {
            Some(pending) => pending.units.saturating_add(u64::from(self.frame_units())),
            None => 0,
        };
        origin + to_ms(units, self.timescale)
    }

    /// Writes the held sample with one frame's duration, the open fragment, and `mfra`/`tfra`,
    /// and syncs the file. Returns its size. The file then reads as complete: every fragment is
    /// there and `mp4::Index::complete_len` equals its length.
    pub fn finish(mut self) -> Result<u64, Error> {
        let duration = self.frame_units();
        if let Some(pending) = self.pending.take() {
            self.push(pending, duration)?;
        }
        self.write_fragment()?;
        if !self.sync_points.is_empty() {
            let mfra = self.mfra();
            self.file.write_all(&mfra)?;
            self.written += mfra.len() as u64;
        }
        self.file.sync_all()?;
        Ok(self.size())
    }

    /// Closes the file without `finish`, as after an error: it stays readable to its last
    /// complete fragment, like a file a hard kill left. Returns its size.
    pub fn abandon(self) -> u64 {
        self.size()
    }

    /// The file's size on disk, as it stands.
    fn size(&self) -> u64 {
        self.file
            .metadata()
            .map(|meta| meta.len())
            .unwrap_or(self.written)
    }

    /// A sample's media time, rounded to the timescale's grid.
    fn units_at(&self, t: u64) -> u64 {
        let Some(origin) = self.origin else {
            return 0;
        };
        let elapsed = u128::from(t.saturating_sub(origin));
        let units = (elapsed * u128::from(self.timescale) + 500) / 1_000;
        u64::try_from(units).unwrap_or(u64::MAX)
    }

    /// One nominal frame in timescale units, the duration of the last sample.
    fn frame_units(&self) -> u32 {
        (self.timescale / FPS).max(1)
    }

    /// Puts a sample whose duration is known into the open fragment, ending that fragment first
    /// when the sample is a sync sample and the fragment is long enough.
    fn push(&mut self, sample: Sample, duration: u32) -> Result<(), Error> {
        if sample.sync {
            match self.open_start {
                Some(start) if sample.units.saturating_sub(start) >= self.min_fragment => {
                    self.write_fragment()?;
                }
                _ => {}
            }
        }
        if self.open_start.is_none() {
            self.open_start = Some(sample.units);
        }
        self.open.push(Written {
            units: sample.units,
            duration,
            sync: sample.sync,
            data: sample.data,
        });
        Ok(())
    }

    /// Writes the open fragment as one `moof`+`mdat` pair: `tfhd` with default-base-is-moof,
    /// `tfdt` version 1, one `trun` with each sample's duration, size and flags, and the `trun`
    /// data offset relative to the `moof`.
    fn write_fragment(&mut self) -> Result<(), Error> {
        if self.open.is_empty() {
            return Ok(());
        }
        let samples = std::mem::take(&mut self.open);
        self.open_start = None;
        self.sequence = self.sequence.saturating_add(1);
        let base = samples[0].units;
        let first_sync = samples[0].sync;

        let mut run = Vec::with_capacity(8 + 12 * samples.len());
        run.extend_from_slice(&fits(samples.len() as u64, "sample count")?.to_be_bytes());
        run.extend_from_slice(&0i32.to_be_bytes()); // the data offset, patched below
        let mut payload = 0u64;
        for sample in &samples {
            let size = fits(sample.data.len() as u64, "sample size")?;
            payload += u64::from(size);
            run.extend_from_slice(&sample.duration.to_be_bytes());
            run.extend_from_slice(&size.to_be_bytes());
            let flags = if sample.sync {
                SYNC_FLAGS
            } else {
                NON_SYNC_FLAGS
            };
            run.extend_from_slice(&flags.to_be_bytes());
        }
        // The mdat's 32-bit size counts its 8-byte header.
        let mdat_size = fits(payload + 8, "fragment size")?;

        let mut trun = bx(b"trun", &full(0, 0x000701, &run));
        let tfhd = bx(b"tfhd", &full(0, 0x020000, &TRACK_ID.to_be_bytes()));
        let tfdt = bx(b"tfdt", &full(1, 0, &base.to_be_bytes()));
        let mfhd = bx(b"mfhd", &full(0, 0, &self.sequence.to_be_bytes()));
        // The data offset is where the mdat's payload lies, counted from the moof's start. A
        // version 0 trun's header is 8 bytes, then version and flags and the sample count.
        let moof_len = 8 + mfhd.len() + 8 + tfhd.len() + tfdt.len() + trun.len();
        let data_offset = i32::try_from(moof_len + 8).map_err(|_| Error::TooLarge {
            what: "fragment header",
            value: moof_len as u64 + 8,
        })?;
        trun[16..20].copy_from_slice(&data_offset.to_be_bytes());
        let traf = bx(b"traf", &[tfhd, tfdt, trun].concat());
        let moof = bx(b"moof", &[mfhd, traf].concat());

        let mut pair = Vec::with_capacity(moof.len() + payload as usize + 8);
        pair.extend_from_slice(&moof);
        pair.extend_from_slice(&mdat_size.to_be_bytes());
        pair.extend_from_slice(b"mdat");
        for sample in &samples {
            pair.extend_from_slice(&sample.data);
        }

        let at = self.written;
        self.file.write_all(&pair)?;
        self.written += pair.len() as u64;
        if first_sync {
            self.sync_points.push(SyncPoint {
                time: base,
                moof: at,
            });
        }
        if self.last_sync.elapsed() >= SYNC_AFTER {
            self.file.sync_data()?;
            self.last_sync = Instant::now();
        }
        Ok(())
    }

    /// `mfra` with one `tfra` entry per fragment that starts on a sync sample: its time, the
    /// offset of its `moof`, and the first `traf`, `trun` and sample, each in one byte.
    fn mfra(&self) -> Vec<u8> {
        let mut payload = Vec::with_capacity(12 + 19 * self.sync_points.len());
        payload.extend_from_slice(&TRACK_ID.to_be_bytes());
        // 26 reserved bits, then the byte counts of the traf, trun and sample numbers minus one.
        payload.extend_from_slice(&0u32.to_be_bytes());
        payload.extend_from_slice(&(self.sync_points.len() as u32).to_be_bytes());
        for point in &self.sync_points {
            payload.extend_from_slice(&point.time.to_be_bytes());
            payload.extend_from_slice(&point.moof.to_be_bytes());
            payload.extend_from_slice(&[1, 1, 1]);
        }
        bx(b"mfra", &bx(b"tfra", &full(1, 0, &payload)))
    }
}

/// The media time in ms, rounded down, as [`crate::mp4::Index::to_ms`] reads it.
fn to_ms(units: u64, timescale: u32) -> u64 {
    let ms = u128::from(units) * 1_000 / u128::from(timescale.max(1));
    u64::try_from(ms).unwrap_or(u64::MAX)
}

/// The `avcC` payload for `sps` and `pps`: version 1, profile, compatibility and level from the
/// SPS, `lengthSizeMinusOne` 3 as AVCC samples have 4-byte lengths, then both sets with their
/// 16-bit lengths.
fn avcc(sps: &[u8], pps: &[u8]) -> Result<Vec<u8>, Error> {
    if sps.len() < 4 {
        return Err(Error::Invalid(
            "the SPS is shorter than the four bytes an avcC box repeats",
        ));
    }
    if pps.is_empty() {
        return Err(Error::Invalid("the PPS is empty"));
    }
    let sps_len = u16::try_from(sps.len()).map_err(|_| Error::TooLarge {
        what: "SPS length",
        value: sps.len() as u64,
    })?;
    let pps_len = u16::try_from(pps.len()).map_err(|_| Error::TooLarge {
        what: "PPS length",
        value: pps.len() as u64,
    })?;
    let mut out = Vec::with_capacity(11 + sps.len() + pps.len());
    out.extend_from_slice(&[1, sps[1], sps[2], sps[3], 0xff, 0xe1]);
    out.extend_from_slice(&sps_len.to_be_bytes());
    out.extend_from_slice(sps);
    out.push(1);
    out.extend_from_slice(&pps_len.to_be_bytes());
    out.extend_from_slice(pps);
    Ok(out)
}

/// `ftyp` with the brands the Windows files carry.
fn ftyp() -> Vec<u8> {
    let mut payload = b"isom".to_vec();
    payload.extend_from_slice(&0x200u32.to_be_bytes());
    payload.extend_from_slice(b"isomiso5iso6avc1mp41");
    bx(b"ftyp", &payload)
}

/// The transformation matrix of an untransformed track.
fn matrix() -> Vec<u8> {
    let mut out = Vec::new();
    for value in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out
}

/// A full box with an empty table: its version and flags and a count of 0.
fn empty_table(kind: &[u8; 4]) -> Vec<u8> {
    bx(kind, &full(0, 0, &0u32.to_be_bytes()))
}

/// `moov` for one H.264 video track: `mvhd`, a `trak` whose sample entry is `avc1` with `avcC`,
/// and `mvex/trex` with the non-sync flag as the default sample flags.
fn moov(timescale: u32, width: u16, height: u16, avcc: &[u8]) -> Vec<u8> {
    let mut mvhd = Vec::new();
    mvhd.extend_from_slice(&[0u8; 8]); // creation, modification
    mvhd.extend_from_slice(&1_000u32.to_be_bytes()); // movie timescale
    mvhd.extend_from_slice(&0u32.to_be_bytes()); // duration
    mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // rate
    mvhd.extend_from_slice(&0x0100u16.to_be_bytes()); // volume
    mvhd.extend_from_slice(&[0u8; 10]);
    mvhd.extend_from_slice(&matrix());
    mvhd.extend_from_slice(&[0u8; 24]); // pre_defined
    mvhd.extend_from_slice(&(TRACK_ID + 1).to_be_bytes()); // next track id
    let mvhd = bx(b"mvhd", &full(0, 0, &mvhd));

    let mut tkhd = Vec::new();
    tkhd.extend_from_slice(&[0u8; 8]);
    tkhd.extend_from_slice(&TRACK_ID.to_be_bytes());
    tkhd.extend_from_slice(&0u32.to_be_bytes()); // reserved
    tkhd.extend_from_slice(&0u32.to_be_bytes()); // duration
    tkhd.extend_from_slice(&[0u8; 8]); // reserved
    tkhd.extend_from_slice(&0u16.to_be_bytes()); // layer
    tkhd.extend_from_slice(&0u16.to_be_bytes()); // alternate group
    tkhd.extend_from_slice(&0u16.to_be_bytes()); // volume
    tkhd.extend_from_slice(&[0u8; 2]);
    tkhd.extend_from_slice(&matrix());
    tkhd.extend_from_slice(&(u32::from(width) << 16).to_be_bytes());
    tkhd.extend_from_slice(&(u32::from(height) << 16).to_be_bytes());
    let tkhd = bx(b"tkhd", &full(0, 0x0000_0007, &tkhd));

    let mut mdhd = Vec::new();
    mdhd.extend_from_slice(&[0u8; 8]);
    mdhd.extend_from_slice(&timescale.to_be_bytes());
    mdhd.extend_from_slice(&0u32.to_be_bytes()); // duration
    mdhd.extend_from_slice(&[0x55, 0xc4, 0, 0]); // language "und"
    let mdhd = bx(b"mdhd", &full(0, 0, &mdhd));
    let hdlr = bx(
        b"hdlr",
        &full(0, 0, b"\0\0\0\0vide\0\0\0\0\0\0\0\0\0\0\0\0VideoHandler\0"),
    );

    let vmhd = bx(b"vmhd", &full(0, 1, &[0u8; 8]));
    let mut dref = full(0, 0, &1u32.to_be_bytes());
    dref.extend_from_slice(&bx(b"url ", &full(0, 1, &[])));
    let dinf = bx(b"dinf", &bx(b"dref", &dref));

    let mut avc1 = Vec::new();
    avc1.extend_from_slice(&[0u8; 6]); // reserved
    avc1.extend_from_slice(&1u16.to_be_bytes()); // data reference index
    avc1.extend_from_slice(&[0u8; 16]); // pre_defined, reserved
    avc1.extend_from_slice(&width.to_be_bytes());
    avc1.extend_from_slice(&height.to_be_bytes());
    avc1.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // 72 dpi
    avc1.extend_from_slice(&0x0048_0000u32.to_be_bytes());
    avc1.extend_from_slice(&0u32.to_be_bytes()); // reserved
    avc1.extend_from_slice(&1u16.to_be_bytes()); // frame count
    avc1.extend_from_slice(&[0u8; 32]); // compressor name
    avc1.extend_from_slice(&0x0018u16.to_be_bytes()); // depth
    avc1.extend_from_slice(&0xffffu16.to_be_bytes()); // pre_defined
    avc1.extend_from_slice(&bx(b"avcC", avcc));

    let mut stsd = full(0, 0, &1u32.to_be_bytes());
    stsd.extend_from_slice(&bx(b"avc1", &avc1));
    let mut stsz = full(0, 0, &0u32.to_be_bytes());
    stsz.extend_from_slice(&0u32.to_be_bytes());
    let stbl = bx(
        b"stbl",
        &[
            bx(b"stsd", &stsd),
            empty_table(b"stts"),
            empty_table(b"stsc"),
            bx(b"stsz", &stsz),
            empty_table(b"stco"),
        ]
        .concat(),
    );
    let minf = bx(b"minf", &[vmhd, dinf, stbl].concat());
    let mdia = bx(b"mdia", &[mdhd, hdlr, minf].concat());
    let trak = bx(b"trak", &[tkhd, mdia].concat());

    let mut trex = Vec::new();
    for value in [TRACK_ID, 1, 0, 0, NON_SYNC_FLAGS] {
        trex.extend_from_slice(&value.to_be_bytes());
    }
    let mvex = bx(b"mvex", &bx(b"trex", &full(0, 0, &trex)));

    bx(b"moov", &[mvhd, trak, mvex].concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4;
    use crate::testdir::TestDir;

    /// The SPS and PPS Apple's H.264 encoder (VideoToolbox) produced for a 320 x 180 frame,
    /// read with `CMVideoFormatDescriptionGetH264ParameterSetAtIndex`: 11 bytes of High profile,
    /// level 1.3, and 4 bytes of PPS. A real encoder's bytes, so the `avcC` box is checked
    /// against what a muxer gets rather than against bytes this test made up.
    const ENCODER_SPS: &[u8] = &[
        0x27, 0x64, 0x00, 0x0d, 0xac, 0x56, 0x50, 0x50, 0x67, 0xe7, 0x40,
    ];
    const ENCODER_PPS: &[u8] = &[0x28, 0xee, 0x3c, 0xb0];

    fn create(path: &Path) -> Mp4Writer {
        Mp4Writer::create(path, TIMESCALE, 320, 180, ENCODER_SPS, ENCODER_PPS).unwrap()
    }

    /// 90 frames at 15 fps, 66.67 ms apart, with a keyframe asked for once the open fragment
    /// holds a second and the first frame always one, as the screen process drives the writer.
    fn write_film(path: &Path) -> Mp4Writer {
        let mut writer = create(path);
        for n in 0..90u64 {
            let t = n * 200 / 3;
            let sync = n == 0 || writer.needs_keyframe(t);
            let data = vec![n as u8; 200 + n as usize];
            writer.add(t, sync, &data).unwrap();
        }
        writer
    }

    fn index_of(path: &Path) -> mp4::Index {
        mp4::index_path(path).unwrap()
    }

    #[test]
    fn fragments_start_on_sync_samples_and_hold_a_second() {
        let dir = TestDir::new("fmp4-fragments");
        let path = dir.path().join("000001.mp4");
        let writer = write_film(&path);
        let end = writer.end();
        let bytes = writer.finish().unwrap();
        let index = index_of(&path);
        assert_eq!(index.timescale, TIMESCALE);
        assert_eq!(index.complete_len, index.file_len);
        assert_eq!(index.complete_len, bytes);
        assert!(index.fragments.len() >= 5, "{:?}", mp4::fragment_ms(&index));
        assert!(index.fragments.iter().all(|f| f.first_sync && f.has_sync));
        let ms = mp4::fragment_ms(&index);
        // Every fragment but the file's last holds at least the second it was closed at; the
        // last is whatever the meeting's end left, as the Windows files' last one is.
        let (last, rest) = ms.split_last().expect("fragments");
        for fragment in rest {
            assert!(
                (1_000..=2_000).contains(fragment),
                "a {fragment} ms fragment"
            );
        }
        assert!(
            *last > 0 && *last <= 2_000,
            "the last fragment is {last} ms"
        );
        let samples: u32 = index.fragments.iter().map(|f| f.samples).sum();
        assert_eq!(samples, 90);
        // `end` before `finish` is where `video_end.t` goes, and the finished file ends there.
        let last = index.fragments.last().unwrap();
        assert_eq!(end, index.to_ms(last.end()));
    }

    #[test]
    fn the_reader_cuts_a_clip_from_the_writers_output() {
        let dir = TestDir::new("fmp4-cut");
        let src = dir.path().join("000001.mp4");
        write_film(&src).finish().unwrap();
        let dst = dir.path().join("clip.mp4");
        let cut = mp4::cut(&src, 1_500, 2_500, &dst).unwrap().unwrap();
        assert!(cut.start_ms <= 1_500 && cut.end_ms >= 2_500, "{cut:?}");
        let index = index_of(&dst);
        assert_eq!(index.complete_len, index.file_len);
        assert!(index.fragments.first().unwrap().first_sync);
        assert_eq!(cut.bytes, index.file_len);
    }

    #[test]
    fn a_torn_last_fragment_is_repaired_by_the_shared_code() {
        let dir = TestDir::new("fmp4-torn");
        let path = dir.path().join("000001.mp4");
        write_film(&path).finish().unwrap();
        let whole = index_of(&path);
        let third = whole.fragments[2];
        let cut_at = (third.offset + third.moof_size + 100) as usize;
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..cut_at]).unwrap();
        let torn = index_of(&path);
        assert_eq!(torn.complete_len, third.offset);
        assert_eq!(torn.fragments.len(), 2);
        let repaired = mp4::repair(&path).unwrap();
        assert_eq!(repaired.kept, third.offset);
        let after = index_of(&path);
        assert_eq!(after.complete_len, after.file_len);
        let dst = dir.path().join("clip.mp4");
        assert!(mp4::cut(&path, 0, 5_000, &dst).unwrap().is_some());
    }

    #[test]
    fn a_gap_becomes_a_longer_sample_and_keeps_the_timeline() {
        let dir = TestDir::new("fmp4-gap");
        let path = dir.path().join("000001.mp4");
        let mut writer = create(&path);
        // The first frame's meeting time is not 0; media time 0 is rebased onto it. The sample
        // at 6 s lasts until 8 s, as a two-second pause on the screen does.
        for (n, (t, sync)) in [
            (5_000u64, true),
            (6_000, true),
            (8_000, true),
            (9_000, false),
        ]
        .into_iter()
        .enumerate()
        {
            writer.add(t, sync, &[n as u8; 64]).unwrap();
        }
        let end = writer.end();
        writer.finish().unwrap();
        let index = index_of(&path);
        assert_eq!(index.complete_len, index.file_len);
        assert_eq!(index.fragments[0].start, 0);
        // One fragment for the 2 s pause, one sample long, and no hole in the timeline.
        assert_eq!(index.fragments[1].samples, 1);
        assert_eq!(index.fragments[1].duration, 30_000);
        let last = index.fragments.last().unwrap();
        assert_eq!(index.to_ms(last.end()), 4_066);
        assert_eq!(end, 5_000 + index.to_ms(last.end()));
        // A range inside the pause finds the fragment that spans it.
        let dst = dir.path().join("clip.mp4");
        let cut = mp4::cut(&path, 1_800, 2_000, &dst).unwrap().unwrap();
        assert_eq!((cut.start_ms, cut.end_ms), (1_000, 3_000));
    }

    /// `avcC` carries what the encoder handed over: version 1, the SPS's profile,
    /// compatibility and level, 4-byte NAL lengths, and both sets with their 16-bit lengths.
    #[test]
    fn the_avcc_holds_the_encoders_parameter_sets() {
        let dir = TestDir::new("fmp4-avcc");
        let path = dir.path().join("000001.mp4");
        create(&path).abandon();
        let bytes = std::fs::read(&path).unwrap();
        let (_, start, end) = boxes(&bytes)
            .into_iter()
            .find(|(kind, ..)| kind == b"moov")
            .unwrap();
        let moov = &bytes[start..end];
        let at = find(moov, b"avcC").expect("an avcC in the sample entry");
        let size = u32::from_be_bytes(moov[at - 8..at - 4].try_into().unwrap()) as usize;
        let avcc = &moov[at..at - 8 + size];
        assert_eq!(avcc[0], 1, "the configuration record's version");
        assert_eq!(&avcc[1..4], &ENCODER_SPS[1..4]);
        assert_eq!(avcc[4], 0xff, "lengthSizeMinusOne 3 and reserved bits");
        assert_eq!(avcc[5], 0xe1, "one SPS and the reserved bits");
        let sps_len = usize::from(u16::from_be_bytes([avcc[6], avcc[7]]));
        assert_eq!(&avcc[8..8 + sps_len], ENCODER_SPS);
        let pps_at = 8 + sps_len;
        assert_eq!(avcc[pps_at], 1, "one PPS");
        let pps_len = usize::from(u16::from_be_bytes([avcc[pps_at + 1], avcc[pps_at + 2]]));
        assert_eq!(&avcc[pps_at + 3..pps_at + 3 + pps_len], ENCODER_PPS);
        assert_eq!(pps_at + 3 + pps_len, avcc.len(), "nothing after the PPS");

        // A parameter set that can't be described is refused before the file is created.
        let short = dir.path().join("short.mp4");
        let refused = Mp4Writer::create(&short, TIMESCALE, 320, 180, &[0x27, 0x64], &[]);
        assert!(
            matches!(&refused, Err(Error::Invalid(_))),
            "{:?}",
            refused.as_ref().err()
        );
        assert!(!short.exists());
    }

    #[test]
    fn a_file_that_ends_after_moov_reads_as_empty() {
        let dir = TestDir::new("fmp4-head-only");
        let path = dir.path().join("000001.mp4");
        let writer = create(&path);
        assert_eq!(writer.end(), 0);
        let bytes = writer.abandon();
        let index = index_of(&path);
        assert!(index.fragments.is_empty());
        assert_eq!(index.duration_ms(), 0);
        assert_eq!(index.ftyp.end - index.ftyp.start, ftyp().len() as u64);
        assert!(index.moov.end > index.moov.start);
        assert_eq!(index.complete_len, index.file_len);
        assert_eq!(index.complete_len, bytes);
        let dst = dir.path().join("clip.mp4");
        assert_eq!(mp4::cut(&path, 0, 1_000, &dst).unwrap(), None);
    }

    #[test]
    fn a_growing_file_is_readable_between_fragments() {
        let dir = TestDir::new("fmp4-growing");
        let path = dir.path().join("000001.mp4");
        let mut writer = create(&path);
        for n in 0..32u64 {
            let t = n * 200 / 3;
            let sync = n == 0 || writer.needs_keyframe(t);
            writer.add(t, sync, &[n as u8; 300]).unwrap();
        }
        // Two fragments are complete while the third sample group is still open, and the file
        // reads and cuts from them before `finish`.
        let growing = index_of(&path);
        assert_eq!(growing.fragments.len(), 2, "{growing:?}");
        assert_eq!(growing.complete_len, growing.file_len);
        let dst = dir.path().join("clip.mp4");
        let cut = mp4::cut(&path, 500, 1_500, &dst).unwrap().unwrap();
        assert!(cut.start_ms <= 500 && cut.end_ms >= 1_500, "{cut:?}");
        for n in 32..90u64 {
            let t = n * 200 / 3;
            let sync = n == 0 || writer.needs_keyframe(t);
            writer.add(t, sync, &[n as u8; 300]).unwrap();
        }
        writer.finish().unwrap();
        let full = index_of(&path);
        assert_eq!(full.complete_len, full.file_len);
        let samples: u32 = full.fragments.iter().map(|f| f.samples).sum();
        assert_eq!(samples, 90);
    }

    #[test]
    fn a_sample_over_a_32_bit_field_is_refused() {
        let dir = TestDir::new("fmp4-too-large");
        let path = dir.path().join("000001.mp4");
        let mut writer = create(&path);
        writer.add(0, true, &[0; 16]).unwrap();
        // A sample 79 hours long: its duration doesn't fit the trun's 32-bit field.
        let err = writer.add(5_000_000_000, false, &[0; 16]).unwrap_err();
        assert!(matches!(&err, Error::TooLarge { .. }), "{err}");
        // Nothing changed: the held sample still ends one nominal frame in.
        assert_eq!(writer.end(), 66);
        // A sample past 4 GiB can't be allocated in a test, so this checks the guard `add` uses
        // on every sample before it stores it.
        let past = u64::from(u32::MAX) + 1;
        assert!(matches!(
            fits(past, "sample size"),
            Err(Error::TooLarge { .. })
        ));
        assert!(fits(u64::from(u32::MAX), "sample size").is_ok());
        writer.finish().unwrap();
        let index = index_of(&path);
        assert_eq!(index.complete_len, index.file_len);
    }

    #[test]
    fn finish_writes_the_mfra_the_reader_accepts() {
        let dir = TestDir::new("fmp4-mfra");
        let path = dir.path().join("000001.mp4");
        write_film(&path).finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let index = index_of(&path);
        assert_eq!(index.complete_len, bytes.len() as u64);
        assert_eq!(mp4::repair(&path).unwrap().cut, 0);
        let top = boxes(&bytes);
        assert_eq!(top.last().unwrap().0, *b"mfra");
        let (_, start, end) = top.into_iter().find(|(kind, ..)| kind == b"mfra").unwrap();
        let mfra = &bytes[start..end];
        let (kind, start, end) = boxes(mfra).into_iter().next().unwrap();
        assert_eq!(&kind, b"tfra");
        let tfra = &mfra[start..end];
        assert_eq!(tfra[0], 1, "version 1 has 64-bit times and offsets");
        assert_eq!(u32::from_be_bytes(tfra[4..8].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_be_bytes(tfra[8..12].try_into().unwrap()),
            0,
            "one byte for each of the traf, trun and sample numbers"
        );
        let count = u32::from_be_bytes(tfra[12..16].try_into().unwrap()) as usize;
        assert_eq!(count, index.fragments.len());
        for (n, fragment) in index.fragments.iter().enumerate() {
            let entry = 16 + 19 * n;
            let time = u64::from_be_bytes(tfra[entry..entry + 8].try_into().unwrap());
            let moof = u64::from_be_bytes(tfra[entry + 8..entry + 16].try_into().unwrap());
            assert_eq!(time, fragment.start);
            assert_eq!(moof, fragment.offset);
            assert_eq!(&tfra[entry + 16..entry + 19], &[1, 1, 1]);
        }
    }

    /// A fragment ends only at a sync sample, and the keyframe the writer asks for is not
    /// repeated for the sample after it.
    #[test]
    fn a_keyframe_is_asked_for_once_a_fragment_holds_a_second() {
        let dir = TestDir::new("fmp4-keyframe");
        let path = dir.path().join("000001.mp4");
        let mut writer = create(&path);
        writer.add(0, true, &[0; 8]).unwrap();
        assert!(!writer.needs_keyframe(999));
        assert!(writer.needs_keyframe(1_000));
        // The forced keyframe at 1 s will start the next fragment: no second ask after it.
        writer.add(1_000, true, &[1; 8]).unwrap();
        assert!(!writer.needs_keyframe(1_066));
        writer.add(1_066, false, &[2; 8]).unwrap();
        assert!(!writer.needs_keyframe(1_133));
        writer.finish().unwrap();
        let index = index_of(&path);
        assert_eq!(index.fragments.len(), 2);
        assert!(index.fragments.iter().all(|f| f.first_sync));
    }

    /// The boxes the reader and the cutter depend on: a `trex` in the `moov`, and a `tfhd` with
    /// `default-base-is-moof`, a version 1 `tfdt` and one `trun` whose data offset points at the
    /// `mdat` payload. The reader accepts a file without these too, so this is the only check
    /// that the cutter's moof-relative copying stays valid.
    #[test]
    fn the_fragment_boxes_carry_what_the_reader_needs() {
        let dir = TestDir::new("fmp4-boxes");
        let path = dir.path().join("000001.mp4");
        write_film(&path).finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let index = index_of(&path);
        let first = index.fragments[0];
        let moof = &bytes[first.offset as usize..(first.offset + first.moof_size) as usize];
        let word = |at: usize| u32::from_be_bytes(moof[at..at + 4].try_into().unwrap());
        let flags = |at: usize| word(at) & 0x00ff_ffff;

        let tfhd = find(moof, b"tfhd").expect("a tfhd");
        assert_eq!(
            flags(tfhd),
            0x020000,
            "default-base-is-moof, so the trun's offset is relative to the moof"
        );
        let tfdt = find(moof, b"tfdt").expect("a tfdt");
        assert_eq!(moof[tfdt], 1, "tfdt version 1");
        assert_eq!(
            u64::from_be_bytes(moof[tfdt + 4..tfdt + 12].try_into().unwrap()),
            first.start
        );
        let trun = find(moof, b"trun").expect("a trun");
        assert_eq!(flags(trun), 0x000701, "duration, size and flags per sample");
        assert_eq!(word(trun + 4), first.samples);
        assert_eq!(
            i32::from_be_bytes(moof[trun + 8..trun + 12].try_into().unwrap()),
            first.moof_size as i32 + 8,
            "the data offset points at the mdat's payload"
        );

        let (_, start, end) = boxes(&bytes)
            .into_iter()
            .find(|(kind, ..)| kind == b"moov")
            .unwrap();
        let moov = &bytes[start..end];
        let trex = find(moov, b"trex").expect("a trex in the mvex");
        assert_eq!(
            u32::from_be_bytes(moov[trex + 4..trex + 8].try_into().unwrap()),
            1
        );
        assert_eq!(
            u32::from_be_bytes(moov[trex + 20..trex + 24].try_into().unwrap()),
            NON_SYNC_FLAGS,
            "the default sample flags mark samples non-sync"
        );
    }

    /// The top-level boxes of `bytes`: their kind, and where their payload starts and ends.
    fn boxes(bytes: &[u8]) -> Vec<([u8; 4], usize, usize)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at + 8 <= bytes.len() {
            let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
            if size < 8 || at + size > bytes.len() {
                break;
            }
            out.push((kind, at + 8, at + size));
            at += size;
        }
        out
    }

    /// The payload start of the first box of `kind` in `bytes`, which a test uses to find the
    /// `avcC` inside a `moov`.
    fn find(bytes: &[u8], kind: &[u8; 4]) -> Option<usize> {
        bytes
            .windows(8)
            .position(|window| &window[4..] == kind)
            .map(|at| at + 8)
    }
}
