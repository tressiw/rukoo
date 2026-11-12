//! A box walker for the fragmented MP4 files the screen process writes: index, repair and cut.
//!
//! Only complete `moof`+`mdat` pairs count. A cut starts at the latest fragment at or before
//! its start whose first sample is a sync sample, copies `ftyp`, `moov` and the fragments,
//! renumbers `mfhd` from 1 and rewrites each `tfhd.base_data_offset` for the new file.
//!
//! Media Foundation's files (map-platform 2): `ftyp`, a `uuid`, `pdin`, `moov` with `mvex/trex`,
//! then `moof`+`mdat` pairs, and `mfra` only after `Finalize`. Each `traf` has a `tfhd` with an
//! absolute base data offset and a `trun` with per-sample durations, sizes and flags, and no
//! `tfdt`, so a fragment starts where the durations before it end.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::Path;
use std::time::Instant;

/// One `moof`+`mdat` pair. `start` and `duration` are in the track's timescale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fragment {
    pub offset: u64,
    pub moof_size: u64,
    pub mdat_size: u64,
    pub start: u64,
    pub duration: u64,
    pub samples: u32,
    pub first_sync: bool,
    pub has_sync: bool,
}

impl Fragment {
    /// The first byte after its `mdat`.
    pub fn end_offset(&self) -> u64 {
        self.offset + self.moof_size + self.mdat_size
    }

    /// Its bytes: `moof` and `mdat`.
    pub fn bytes(&self) -> u64 {
        self.moof_size + self.mdat_size
    }

    /// Where its media time ends, in the track's timescale.
    pub fn end(&self) -> u64 {
        self.start.saturating_add(self.duration)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    pub timescale: u32,
    pub ftyp: Range<u64>,
    pub moov: Range<u64>,
    pub fragments: Vec<Fragment>,
    /// The end of the last complete box.
    pub complete_len: u64,
    pub file_len: u64,
}

impl Index {
    /// Media time in ms, rounded down.
    pub fn to_ms(&self, units: u64) -> u64 {
        scale(units, 1_000, u64::from(self.timescale.max(1)))
    }

    /// Media time in ms, rounded up.
    pub fn to_ms_ceil(&self, units: u64) -> u64 {
        let ts = u64::from(self.timescale.max(1));
        let ms = scale(units, 1_000, ts);
        if scale(ms, ts, 1_000) < units {
            ms + 1
        } else {
            ms
        }
    }

    /// The end of the last complete fragment in ms of media time, or 0 without one.
    pub fn duration_ms(&self) -> u64 {
        self.fragments
            .last()
            .map_or(0, |f| self.to_ms_ceil(f.end()))
    }

    fn units(&self, ms: u64) -> u64 {
        scale(ms, u64::from(self.timescale.max(1)), 1_000)
    }
}

/// `value * mul / div` without overflow, rounded down.
fn scale(value: u64, mul: u64, div: u64) -> u64 {
    (u128::from(value) * u128::from(mul) / u128::from(div.max(1))) as u64
}

/// The largest `moov` or `moof` read into memory; anything larger isn't one of ours.
const MAX_HEAD_BOX: u64 = 16 << 20;
/// `trun` sample flags: the sample is not a sync sample.
const NON_SYNC: u32 = 0x0001_0000;

/// A box header: its type, where it starts, the header's size and the whole box's size.
#[derive(Clone, Copy, Debug)]
struct Header {
    kind: [u8; 4],
    offset: u64,
    size: u64,
}

impl Header {
    /// `read_header` checked that this doesn't overflow.
    fn end(&self) -> u64 {
        self.offset + self.size
    }
}

/// The header of the box at `offset`, or `None` when no complete, sane header is there. A size
/// of 0 ("to the end of the file") counts as incomplete: the screen process never writes it, and
/// in a growing file it can't be told from a box still being written. Neither is a size whose
/// end lies past what a `u64` holds, which only a damaged or crafted file has: walking on from
/// a wrapped end would loop forever.
fn read_header(file: &mut (impl Read + Seek), offset: u64, len: u64) -> io::Result<Option<Header>> {
    if offset.checked_add(8).is_none_or(|end| end > len) {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut head = [0u8; 8];
    file.read_exact(&mut head)?;
    let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
    let kind = [head[4], head[5], head[6], head[7]];
    let (size, header) = match size {
        0 => return Ok(None),
        1 => {
            if offset.checked_add(16).is_none_or(|end| end > len) {
                return Ok(None);
            }
            let mut large = [0u8; 8];
            file.read_exact(&mut large)?;
            (u64::from_be_bytes(large), 16)
        }
        n => (u64::from(n), 8),
    };
    if size < header || offset.checked_add(size).is_none() {
        return Ok(None);
    }
    Ok(Some(Header { kind, offset, size }))
}

fn read_box(file: &mut (impl Read + Seek), h: &Header) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(h.offset))?;
    let mut buf = vec![0u8; h.size as usize];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

fn be32(buf: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(buf.get(at..at + 4)?.try_into().ok()?))
}

fn be64(buf: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(buf.get(at..at + 8)?.try_into().ok()?))
}

/// The child boxes in `buf[start..end]`: type, start of the box and of its payload, end.
fn children(buf: &[u8], start: usize, end: usize) -> Vec<([u8; 4], usize, usize, usize)> {
    let mut out = Vec::new();
    let mut at = start;
    while at + 8 <= end {
        let Some(size) = be32(buf, at) else { break };
        let kind: [u8; 4] = buf[at + 4..at + 8].try_into().unwrap();
        let (size, header) = match size {
            1 => match be64(buf, at + 8).and_then(|large| usize::try_from(large).ok()) {
                Some(large) => (large, 16),
                None => break,
            },
            0 => (end - at, 8),
            n => (n as usize, 8),
        };
        let Some(box_end) = at.checked_add(size).filter(|e| *e <= end) else {
            break;
        };
        if size < header {
            break;
        }
        out.push((kind, at, at + header, box_end));
        at = box_end;
    }
    out
}

fn child(buf: &[u8], start: usize, end: usize, kind: &[u8; 4]) -> Option<(usize, usize, usize)> {
    children(buf, start, end)
        .into_iter()
        .find(|(k, ..)| k == kind)
        .map(|(_, at, payload, end)| (at, payload, end))
}

/// What `moov` says: the video track's timescale and its `trex` defaults.
#[derive(Clone, Copy, Debug, Default)]
struct Movie {
    timescale: u32,
    default_duration: u32,
    default_flags: u32,
}

/// Where the payload of the box at the start of `buf` begins.
fn payload_start(buf: &[u8]) -> usize {
    let header = if be32(buf, 0) == Some(1) { 16 } else { 8 };
    header.min(buf.len())
}

fn parse_moov(buf: &[u8]) -> Movie {
    let mut movie = Movie::default();
    let (start, end) = (payload_start(buf), buf.len());
    if let Some(timescale) = (|| {
        let (_, trak, trak_end) = child(buf, start, end, b"trak")?;
        let (_, mdia, mdia_end) = child(buf, trak, trak_end, b"mdia")?;
        let (_, mdhd, _) = child(buf, mdia, mdia_end, b"mdhd")?;
        let version = *buf.get(mdhd)?;
        be32(buf, mdhd + if version == 1 { 20 } else { 12 })
    })() {
        movie.timescale = timescale;
    }
    if let Some((_, trex, _)) = child(buf, start, end, b"mvex")
        .and_then(|(_, mvex, mvex_end)| child(buf, mvex, mvex_end, b"trex"))
    {
        // version and flags, track_ID, default_sample_description_index, then these.
        movie.default_duration = be32(buf, trex + 12).unwrap_or(0);
        movie.default_flags = be32(buf, trex + 20).unwrap_or(0);
    }
    movie
}

/// What a `moof` holds, from its first `traf`, and where its fields to rewrite sit.
#[derive(Clone, Debug, Default)]
struct Moof {
    /// Offset of `mfhd`'s sequence number in the box.
    sequence_at: Option<usize>,
    /// Offsets of every `tfhd.base_data_offset` in the box.
    base_offsets_at: Vec<usize>,
    decode_time: Option<u64>,
    samples: u32,
    duration: u64,
    first_sync: bool,
    has_sync: bool,
}

fn parse_moof(buf: &[u8], movie: &Movie) -> Option<Moof> {
    let mut moof = Moof::default();
    let end = buf.len();
    let mut first_traf = true;
    for (kind, _, payload, child_end) in children(buf, payload_start(buf), end) {
        match &kind {
            b"mfhd" => moof.sequence_at = Some(payload + 4),
            b"traf" => {
                parse_traf(buf, payload, child_end, movie, &mut moof, first_traf)?;
                first_traf = false;
            }
            _ => {}
        }
    }
    (!first_traf).then_some(moof)
}

fn parse_traf(
    buf: &[u8],
    start: usize,
    end: usize,
    movie: &Movie,
    moof: &mut Moof,
    timing: bool,
) -> Option<()> {
    let (_, tfhd, _) = child(buf, start, end, b"tfhd")?;
    let flags = be32(buf, tfhd)? & 0x00FF_FFFF;
    let mut at = tfhd + 8; // version and flags, track_ID
    if flags & 0x01 != 0 {
        moof.base_offsets_at.push(at);
        at += 8;
    }
    if flags & 0x02 != 0 {
        at += 4;
    }
    let mut default_duration = movie.default_duration;
    if flags & 0x08 != 0 {
        default_duration = be32(buf, at)?;
        at += 4;
    }
    if flags & 0x10 != 0 {
        at += 4;
    }
    let mut default_flags = movie.default_flags;
    if flags & 0x20 != 0 {
        default_flags = be32(buf, at)?;
    }
    if !timing {
        return Some(());
    }
    if let Some((_, tfdt, _)) = child(buf, start, end, b"tfdt") {
        moof.decode_time = Some(if buf.get(tfdt) == Some(&1) {
            be64(buf, tfdt + 4)?
        } else {
            u64::from(be32(buf, tfdt + 4)?)
        });
    }
    for (kind, _, trun, trun_end) in children(buf, start, end) {
        if &kind != b"trun" {
            continue;
        }
        let flags = be32(buf, trun)? & 0x00FF_FFFF;
        let count = be32(buf, trun + 4)?;
        let mut at = trun + 8;
        if flags & 0x001 != 0 {
            at += 4;
        }
        let mut first_flags = None;
        if flags & 0x004 != 0 {
            first_flags = Some(be32(buf, at)?);
            at += 4;
        }
        let per_sample = [0x100, 0x200, 0x400, 0x800]
            .iter()
            .filter(|f| flags & *f != 0)
            .count()
            * 4;
        if per_sample == 0 {
            // Every sample takes the defaults: no bytes to walk, so a huge count costs nothing.
            if count > 0 {
                let first = first_flags.unwrap_or(default_flags) & NON_SYNC == 0;
                let rest = default_flags & NON_SYNC == 0;
                if moof.samples == 0 {
                    moof.first_sync = first;
                }
                moof.has_sync |= first || (count > 1 && rest);
                moof.samples = moof.samples.saturating_add(count);
                moof.duration = moof
                    .duration
                    .saturating_add(u64::from(count) * u64::from(default_duration));
            }
            continue;
        }
        // A count the box can't hold is a damaged box.
        if (count as usize)
            .checked_mul(per_sample)
            .and_then(|bytes| at.checked_add(bytes))
            .is_none_or(|samples_end| samples_end > trun_end)
        {
            return None;
        }
        for i in 0..count {
            let mut duration = default_duration;
            let mut sample_flags = default_flags;
            if flags & 0x100 != 0 {
                duration = be32(buf, at)?;
                at += 4;
            }
            if flags & 0x200 != 0 {
                at += 4;
            }
            if flags & 0x400 != 0 {
                sample_flags = be32(buf, at)?;
                at += 4;
            } else if i == 0 {
                if let Some(first) = first_flags {
                    sample_flags = first;
                }
            }
            if flags & 0x800 != 0 {
                at += 4;
            }
            let sync = sample_flags & NON_SYNC == 0;
            if moof.samples == 0 {
                moof.first_sync = sync;
            }
            moof.has_sync |= sync;
            moof.samples = moof.samples.saturating_add(1);
            moof.duration = moof.duration.saturating_add(u64::from(duration));
        }
    }
    Some(())
}

/// Walks the boxes of `len` bytes; stops at the first incomplete box.
///
/// A `moof` counts once the `mdat` after it is complete; until then `complete_len` stays before
/// it. Other complete boxes, such as `mfra` after `Finalize`, move `complete_len` on.
pub fn index(file: &mut (impl Read + Seek), len: u64) -> io::Result<Index> {
    let mut index = Index {
        timescale: 0,
        ftyp: 0..0,
        moov: 0..0,
        fragments: Vec::new(),
        complete_len: 0,
        file_len: len,
    };
    let mut movie = Movie::default();
    let mut pending: Option<(Header, Moof)> = None;
    let mut next_start = 0u64;
    let mut offset = 0u64;
    while let Some(h) = read_header(file, offset, len)? {
        if h.end() > len {
            break;
        }
        match &h.kind {
            b"ftyp" => index.ftyp = h.offset..h.end(),
            b"moov" if h.size <= MAX_HEAD_BOX => {
                index.moov = h.offset..h.end();
                movie = parse_moov(&read_box(file, &h)?);
                index.timescale = movie.timescale;
            }
            b"moof" if h.size <= MAX_HEAD_BOX => {
                let moof = parse_moof(&read_box(file, &h)?, &movie);
                // A moof without its mdat is dropped when anything else follows it.
                pending = moof.map(|m| (h, m));
                offset = h.end();
                continue;
            }
            b"mdat" => {
                if let Some((moof_header, moof)) = pending.take() {
                    let start = moof.decode_time.unwrap_or(next_start);
                    next_start = start.saturating_add(moof.duration);
                    index.fragments.push(Fragment {
                        offset: moof_header.offset,
                        moof_size: moof_header.size,
                        mdat_size: h.size,
                        start,
                        duration: moof.duration,
                        samples: moof.samples,
                        first_sync: moof.first_sync,
                        has_sync: moof.has_sync,
                    });
                }
            }
            _ => {}
        }
        pending = None;
        index.complete_len = h.end();
        offset = h.end();
    }
    Ok(index)
}

pub fn index_path(path: &Path) -> io::Result<Index> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    index(&mut file, len)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repaired {
    pub kept: u64,
    pub cut: u64,
}

/// Truncates the file to its last complete box: after a hard kill, power loss or a full disk,
/// the file then ends on a complete fragment and plays to its end.
pub fn repair(path: &Path) -> io::Result<Repaired> {
    let index = index_path(path)?;
    let cut = index.file_len - index.complete_len;
    if cut > 0 {
        let file = OpenOptions::new().write(true).open(path)?;
        file.set_len(index.complete_len)?;
        file.sync_all()?;
    }
    Ok(Repaired {
        kept: index.complete_len,
        cut,
    })
}

/// What a cut holds, in ms of the source file's media time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cut {
    pub start_ms: u64,
    pub end_ms: u64,
    pub bytes: u64,
}

/// The fragments a cut of `[from_ms, to_ms]` copies: from the latest one at or before `from_ms`
/// that starts on a sync sample to the last one that starts before `to_ms`. `None` when no
/// complete fragment overlaps the range or none before its end starts on a sync sample.
pub fn select(index: &Index, from_ms: u64, to_ms: u64) -> Option<Range<usize>> {
    let fragments = &index.fragments;
    let (from, to) = (index.units(from_ms), index.units(to_ms.max(from_ms)));
    let first = fragments.iter().position(|f| f.end() > from)?;
    if fragments[first].start > to {
        return None;
    }
    let start = match (0..=first).rev().find(|&i| fragments[i].first_sync) {
        Some(start) => start,
        // Nothing before starts on a keyframe: the first one within the range that does.
        None => (first..fragments.len())
            .take_while(|&i| fragments[i].start < to || i == first)
            .find(|&i| fragments[i].first_sync)?,
    };
    let last = (start..fragments.len())
        .take_while(|&i| fragments[i].start < to || i == start || i <= first)
        .last()?;
    Some(start..last + 1)
}

/// What a cut of `[from_ms, to_ms]` would hold without making it: `bytes` is the size of the
/// file it writes.
pub fn plan(index: &Index, from_ms: u64, to_ms: u64) -> Option<Cut> {
    let range = select(index, from_ms, to_ms)?;
    let fragments = &index.fragments[range];
    let (first, last) = (fragments.first()?, fragments.last()?);
    let head = (index.ftyp.end - index.ftyp.start) + (index.moov.end - index.moov.start);
    Some(Cut {
        start_ms: index.to_ms(first.start),
        end_ms: index.to_ms_ceil(last.end()),
        bytes: head + fragments.iter().map(Fragment::bytes).sum::<u64>(),
    })
}

/// Copies the fragments covering `[from_ms, to_ms]` of `src` into `dst`. `None` when no complete
/// fragment overlaps the range.
pub fn cut(src: &Path, from_ms: u64, to_ms: u64, dst: &Path) -> io::Result<Option<Cut>> {
    cut_until(src, from_ms, to_ms, dst, None)
}

/// `cut`, stopped with `TimedOut` when `deadline` passes between two fragments. A cut that fails
/// or stops removes `dst`. The written file is synced.
pub(crate) fn cut_until(
    src: &Path,
    from_ms: u64,
    to_ms: u64,
    dst: &Path,
    deadline: Option<Instant>,
) -> io::Result<Option<Cut>> {
    let mut source = File::open(src)?;
    let len = source.metadata()?.len();
    let index = index(&mut source, len)?;
    if index.ftyp.is_empty() || index.moov.is_empty() {
        return Ok(None);
    }
    let Some(range) = select(&index, from_ms, to_ms) else {
        return Ok(None);
    };
    let fragments = index.fragments[range].to_vec();
    let movie = {
        let moov = Header {
            kind: *b"moov",
            offset: index.moov.start,
            size: index.moov.end - index.moov.start,
        };
        parse_moov(&read_box(&mut source, &moov)?)
    };
    let result = write_cut(&mut source, &index, &fragments, &movie, dst, deadline);
    if result.is_err() {
        let _ = std::fs::remove_file(dst);
    }
    let bytes = result?;
    let (first, last) = (fragments[0], fragments[fragments.len() - 1]);
    Ok(Some(Cut {
        start_ms: index.to_ms(first.start),
        end_ms: index.to_ms_ceil(last.end()),
        bytes,
    }))
}

fn write_cut(
    source: &mut File,
    index: &Index,
    fragments: &[Fragment],
    movie: &Movie,
    dst: &Path,
    deadline: Option<Instant>,
) -> io::Result<u64> {
    let mut out = File::create(dst)?;
    let mut written = 0u64;
    for range in [&index.ftyp, &index.moov] {
        source.seek(SeekFrom::Start(range.start))?;
        written += io::copy(&mut (&mut *source).take(range.end - range.start), &mut out)?;
    }
    for (n, f) in fragments.iter().enumerate() {
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the cut ran out of time",
            ));
        }
        let header = Header {
            kind: *b"moof",
            offset: f.offset,
            size: f.moof_size,
        };
        let mut moof = read_box(source, &header)?;
        let parsed = parse_moof(&moof, movie)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a moof changed"))?;
        if let Some(at) = parsed.sequence_at {
            moof[at..at + 4].copy_from_slice(&(n as u32 + 1).to_be_bytes());
        }
        // Base data offsets are file positions: move them by as much as the moof moved.
        for at in parsed.base_offsets_at {
            let old = be64(&moof, at).unwrap_or(0);
            let moved = old.wrapping_sub(f.offset).wrapping_add(written);
            moof[at..at + 8].copy_from_slice(&moved.to_be_bytes());
        }
        out.write_all(&moof)?;
        source.seek(SeekFrom::Start(f.offset + f.moof_size))?;
        let copied = io::copy(&mut (&mut *source).take(f.mdat_size), &mut out)?;
        if copied != f.mdat_size {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        written += f.moof_size + f.mdat_size;
    }
    out.sync_all()?;
    Ok(written)
}

/// Each fragment's duration in ms.
pub fn fragment_ms(index: &Index) -> Vec<u64> {
    index
        .fragments
        .iter()
        .map(|f| index.to_ms(f.end()) - index.to_ms(f.start))
        .collect()
}

/// A box: its 32-bit size, its type, then its payload.
pub fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(&(payload.len() as u32 + 8).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

/// A full box's payload: version, flags, then `rest`.
pub fn full(version: u8, flags: u32, rest: &[u8]) -> Vec<u8> {
    let mut out = vec![version];
    out.extend_from_slice(&flags.to_be_bytes()[1..]);
    out.extend_from_slice(rest);
    out
}

/// Builds fragmented MP4 files the way Media Foundation lays them out, for tests here and in
/// other crates, from the same box builders as the writer in [`crate::fmp4`].
#[doc(hidden)]
pub mod synth {
    pub use super::{bx, full};

    /// One sample: duration in timescale units, size in bytes, whether it is a sync sample.
    #[derive(Clone, Copy, Debug)]
    pub struct Sample {
        pub duration: u32,
        pub size: u32,
        pub sync: bool,
    }

    pub fn ftyp() -> Vec<u8> {
        let mut payload = b"mp42".to_vec();
        payload.extend_from_slice(&1u32.to_be_bytes());
        payload.extend_from_slice(b"mp41mp42isom");
        bx(b"ftyp", &payload)
    }

    /// `moov` with one video track at `timescale` and `trex` defaults marking samples non-sync.
    pub fn moov(timescale: u32) -> Vec<u8> {
        let mut mdhd = Vec::new();
        mdhd.extend_from_slice(&[0u8; 8]); // creation, modification
        mdhd.extend_from_slice(&timescale.to_be_bytes());
        mdhd.extend_from_slice(&0u32.to_be_bytes()); // duration
        mdhd.extend_from_slice(&[0x55, 0xc4, 0, 0]);
        let mdhd = bx(b"mdhd", &full(0, 0, &mdhd));
        let hdlr = bx(
            b"hdlr",
            &full(0, 0, b"\0\0\0\0vide\0\0\0\0\0\0\0\0\0\0\0\0\0"),
        );
        let mdia = bx(b"mdia", &[mdhd, hdlr].concat());
        let mut tkhd = vec![0u8; 80];
        tkhd[8..12].copy_from_slice(&1u32.to_be_bytes());
        let trak = bx(b"trak", &[bx(b"tkhd", &full(0, 1, &tkhd)), mdia].concat());
        let mut trex = Vec::new();
        for value in [1u32, 1, 0, 0, 0x0001_0000] {
            trex.extend_from_slice(&value.to_be_bytes());
        }
        let mvex = bx(b"mvex", &bx(b"trex", &full(0, 0, &trex)));
        let mut mvhd = vec![0u8; 96];
        mvhd[8..12].copy_from_slice(&1000u32.to_be_bytes());
        bx(
            b"moov",
            &[bx(b"mvhd", &full(0, 0, &mvhd)), trak, mvex].concat(),
        )
    }

    /// A `moof`+`mdat` pair at file offset `at`, numbered `sequence`, with an absolute base data
    /// offset at the `mdat` payload, as Media Foundation writes it. The payload bytes are `fill`.
    pub fn fragment(at: u64, sequence: u32, samples: &[Sample], fill: u8) -> Vec<u8> {
        let mut trun = Vec::new();
        trun.extend_from_slice(&(samples.len() as u32).to_be_bytes());
        trun.extend_from_slice(&0u32.to_be_bytes()); // data offset
        for s in samples {
            trun.extend_from_slice(&s.duration.to_be_bytes());
            trun.extend_from_slice(&s.size.to_be_bytes());
            let flags: u32 = if s.sync { 0x0200_0000 } else { 0x0001_0000 };
            trun.extend_from_slice(&flags.to_be_bytes());
            trun.extend_from_slice(&0u32.to_be_bytes());
        }
        let trun = bx(b"trun", &full(1, 0x000f01, &trun));
        let moof_len = 8 + 16 + 8 + 24 + trun.len();
        let base = at + moof_len as u64 + 8;
        let mut tfhd = 1u32.to_be_bytes().to_vec();
        tfhd.extend_from_slice(&base.to_be_bytes());
        let traf = bx(b"traf", &[bx(b"tfhd", &full(0, 1, &tfhd)), trun].concat());
        let mfhd = bx(b"mfhd", &full(0, 0, &sequence.to_be_bytes()));
        let moof = bx(b"moof", &[mfhd, traf].concat());
        assert_eq!(moof.len(), moof_len);
        let payload: usize = samples.iter().map(|s| s.size as usize).sum();
        [moof, bx(b"mdat", &vec![fill; payload])].concat()
    }

    /// A whole file: `ftyp`, a `uuid`, `moov`, then one fragment per entry of `fragments`.
    pub fn file(timescale: u32, fragments: &[Vec<Sample>]) -> Vec<u8> {
        let mut out = [ftyp(), bx(b"uuid", &[0x5c; 16]), moov(timescale)].concat();
        for (i, samples) in fragments.iter().enumerate() {
            let pair = fragment(
                out.len() as u64,
                i as u32 + 1,
                samples,
                (i as u8).wrapping_add(1),
            );
            out.extend_from_slice(&pair);
        }
        out
    }

    /// `n` samples of `duration` units, the first a sync sample when `keyframe`.
    pub fn samples(n: usize, duration: u32, size: u32, keyframe: bool) -> Vec<Sample> {
        (0..n)
            .map(|i| Sample {
                duration,
                size,
                sync: keyframe && i == 0,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::synth::*;
    use super::*;
    use crate::testdir::TestDir;

    const TS: u32 = 30_000;
    /// One second at 15 fps in 30 kHz units.
    const FRAME: u32 = 2_000;

    /// Six 1 s fragments, each starting on a keyframe except the fourth (3 to 4 s).
    fn six_seconds() -> Vec<Vec<Sample>> {
        (0..6)
            .map(|i| samples(15, FRAME, 100 + i as u32, i != 3))
            .collect()
    }

    fn index_bytes(bytes: &[u8]) -> Index {
        index(&mut Cursor::new(bytes), bytes.len() as u64).unwrap()
    }

    #[test]
    fn indexes_a_complete_file() {
        let bytes = file(TS, &six_seconds());
        let index = index_bytes(&bytes);
        assert_eq!(index.timescale, TS);
        assert_eq!(index.ftyp, 0..ftyp().len() as u64);
        assert_eq!(index.moov.end - index.moov.start, moov(TS).len() as u64);
        assert_eq!(index.fragments.len(), 6);
        assert_eq!(index.complete_len, bytes.len() as u64);
        assert_eq!(index.file_len, bytes.len() as u64);
        let starts: Vec<u64> = index.fragments.iter().map(|f| f.start).collect();
        assert_eq!(starts, vec![0, 30_000, 60_000, 90_000, 120_000, 150_000]);
        let syncs: Vec<bool> = index.fragments.iter().map(|f| f.first_sync).collect();
        assert_eq!(syncs, vec![true, true, true, false, true, true]);
        assert!(index
            .fragments
            .iter()
            .enumerate()
            .all(|(i, f)| f.has_sync == (i != 3)));
        assert!(index
            .fragments
            .iter()
            .all(|f| f.samples == 15 && f.duration == 30_000));
        assert_eq!(fragment_ms(&index), vec![1_000; 6]);
        assert_eq!(index.duration_ms(), 6_000);
        // Fragments sit back to back after the head.
        for pair in index.fragments.windows(2) {
            assert_eq!(pair[0].end_offset(), pair[1].offset);
        }
        assert_eq!(index.fragments[5].end_offset(), bytes.len() as u64);
    }

    #[test]
    fn mfra_after_finalize_is_kept_as_a_complete_box() {
        let mut bytes = file(TS, &six_seconds()[..2]);
        let fragments_end = bytes.len() as u64;
        bytes.extend(bx(b"mfra", &[0u8; 32]));
        let index = index_bytes(&bytes);
        assert_eq!(index.fragments.len(), 2);
        assert_eq!(index.fragments[1].end_offset(), fragments_end);
        assert_eq!(index.complete_len, bytes.len() as u64);
    }

    #[test]
    fn stops_at_a_torn_moof_or_mdat() {
        let bytes = file(TS, &six_seconds());
        let whole = index_bytes(&bytes);
        let fifth = whole.fragments[4];
        // Torn inside the fifth fragment's moof.
        let torn = &bytes[..(fifth.offset + 20) as usize];
        let index = index_bytes(torn);
        assert_eq!(index.fragments.len(), 4);
        assert_eq!(index.complete_len, fifth.offset);
        assert_eq!(index.file_len, torn.len() as u64);
        // Torn inside its mdat: the moof alone doesn't count.
        let torn = &bytes[..(fifth.offset + fifth.moof_size + 10) as usize];
        let index = index_bytes(torn);
        assert_eq!(index.fragments.len(), 4);
        assert_eq!(index.complete_len, fifth.offset);
        // Torn in the middle of a box header.
        let torn = &bytes[..(fifth.offset + 3) as usize];
        assert_eq!(index_bytes(torn).complete_len, fifth.offset);
        // Only the head written yet, or not even that.
        let head = whole.fragments[0].offset as usize;
        let index = index_bytes(&bytes[..head]);
        assert!(index.fragments.is_empty());
        assert_eq!(index.complete_len, head as u64);
        assert_eq!(index.duration_ms(), 0);
        let ftyp_len = ftyp().len();
        assert_eq!(
            index_bytes(&bytes[..ftyp_len + 6]).complete_len,
            ftyp_len as u64
        );
        assert_eq!(index_bytes(&[]).complete_len, 0);
        // A moof whose mdat never came, followed by more boxes, isn't a fragment.
        let mut odd = bytes[..fifth.offset as usize].to_vec();
        odd.extend(&bytes[fifth.offset as usize..(fifth.offset + fifth.moof_size) as usize]);
        odd.extend(bx(b"free", &[0; 4]));
        assert_eq!(index_bytes(&odd).fragments.len(), 4);
    }

    /// A box whose 64-bit size runs past what a `u64` holds ends the walk; walking on from the
    /// wrapped end went back to an earlier box and looped forever.
    #[test]
    fn a_size_that_wraps_ends_the_walk() {
        // A 16-byte ftyp whose payload looks like an 8-byte free box, then a size-1 free box
        // with largesize 2^64 - 8: its end wraps to 8, inside the ftyp.
        let mut bytes = 16u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(b"free");
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(b"free");
        bytes.extend_from_slice(&(u64::MAX - 7).to_be_bytes());
        bytes.extend_from_slice(&[0u8; 16]);
        assert_eq!(bytes.len(), 48);
        let (done, result) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(index(&mut Cursor::new(&bytes), 48).map(|i| i.complete_len));
        });
        let complete = result
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the walk didn't end")
            .unwrap();
        assert_eq!(complete, 16);

        // The same inside a moof: a child box whose size wraps ends the children.
        let mut buf = 24u32.to_be_bytes().to_vec();
        buf.extend_from_slice(b"moof");
        buf.extend_from_slice(&1u32.to_be_bytes());
        buf.extend_from_slice(b"traf");
        buf.extend_from_slice(&(u64::MAX - 7).to_be_bytes());
        assert!(children(&buf, 8, buf.len()).is_empty());
    }

    /// A `trun` that claims four billion samples is read without a walk when every sample takes
    /// the defaults, and refused when its samples can't fit in the box.
    #[test]
    fn a_huge_sample_count_costs_nothing() {
        let movie = Movie {
            timescale: TS,
            default_duration: FRAME,
            default_flags: NON_SYNC,
        };
        let trun = |flags: u32| {
            let mut payload = u32::MAX.to_be_bytes().to_vec();
            payload.extend_from_slice(&0u32.to_be_bytes()); // data offset
            bx(b"trun", &full(0, flags, &payload))
        };
        let tfhd = bx(b"tfhd", &full(0, 0, &1u32.to_be_bytes()));
        let moof = |trun: Vec<u8>| bx(b"moof", &bx(b"traf", &[tfhd.clone(), trun].concat()));
        let started = Instant::now();
        let parsed = parse_moof(&moof(trun(0x000001)), &movie).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert_eq!(parsed.samples, u32::MAX);
        assert_eq!(parsed.duration, u64::from(u32::MAX) * u64::from(FRAME));
        assert!(!parsed.first_sync && !parsed.has_sync);
        // Per-sample durations promised for four billion samples in 16 bytes: damaged.
        assert!(parse_moof(&moof(trun(0x000101)), &movie).is_none());
    }

    #[test]
    fn repair_cuts_back_to_the_last_complete_fragment() {
        let dir = TestDir::new("mp4-repair");
        let path = dir.path().join("000001.mp4");
        let bytes = file(TS, &six_seconds());
        let whole = index_bytes(&bytes);
        let cut_at = (whole.fragments[3].offset + whole.fragments[3].moof_size + 7) as usize;
        std::fs::write(&path, &bytes[..cut_at]).unwrap();
        let repaired = repair(&path).unwrap();
        assert_eq!(
            repaired,
            Repaired {
                kept: whole.fragments[3].offset,
                cut: cut_at as u64 - whole.fragments[3].offset,
            }
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            &bytes[..whole.fragments[3].offset as usize]
        );
        // A second repair, or one of a complete file, changes nothing.
        assert_eq!(repair(&path).unwrap().cut, 0);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(
            repair(&path).unwrap(),
            Repaired {
                kept: bytes.len() as u64,
                cut: 0
            }
        );
        assert!(repair(&dir.path().join("missing.mp4")).is_err());
    }

    /// The fragment numbers and base offsets of a cut, checked against its own layout.
    fn assert_well_formed(bytes: &[u8]) -> Index {
        let index = index_bytes(bytes);
        assert_eq!(index.complete_len, bytes.len() as u64);
        let movie = parse_moov(&bytes[index.moov.start as usize..index.moov.end as usize]);
        for (n, f) in index.fragments.iter().enumerate() {
            let moof = &bytes[f.offset as usize..(f.offset + f.moof_size) as usize];
            let parsed = parse_moof(moof, &movie).unwrap();
            let sequence = be32(moof, parsed.sequence_at.unwrap()).unwrap();
            assert_eq!(sequence, n as u32 + 1);
            for at in parsed.base_offsets_at {
                // Each base offset still points at its own mdat's payload.
                assert_eq!(be64(moof, at).unwrap(), f.offset + f.moof_size + 8);
            }
        }
        index
    }

    #[test]
    fn cut_copies_whole_fragments_from_a_keyframe() {
        let dir = TestDir::new("mp4-cut");
        let src = dir.path().join("000001.mp4");
        let bytes = file(TS, &six_seconds());
        std::fs::write(&src, &bytes).unwrap();
        let source = index_bytes(&bytes);
        let dst = dir.path().join("clip.mp4");

        // 1.5 s to 2.5 s: the second and third fragments.
        let cut = cut(&src, 1_500, 2_500, &dst).unwrap().unwrap();
        assert_eq!((cut.start_ms, cut.end_ms), (1_000, 3_000));
        let out = std::fs::read(&dst).unwrap();
        assert_eq!(cut.bytes, out.len() as u64);
        let index = assert_well_formed(&out);
        assert_eq!(index.fragments.len(), 2);
        assert_eq!(index.fragments[0].start, 0);
        // The head is copied as is, and so is every mdat.
        let head = [
            &bytes[source.ftyp.start as usize..source.ftyp.end as usize],
            &bytes[source.moov.start as usize..source.moov.end as usize],
        ]
        .concat();
        assert_eq!(&out[..head.len()], &head[..]);
        for (f, g) in index.fragments.iter().zip(&source.fragments[1..3]) {
            let mdat = |b: &[u8], f: &Fragment| {
                b[(f.offset + f.moof_size) as usize..f.end_offset() as usize].to_vec()
            };
            assert_eq!(mdat(&out, f), mdat(&bytes, g));
            assert_eq!(f.samples, g.samples);
        }
        assert_eq!(plan(&source, 1_500, 2_500), Some(cut));
    }

    #[test]
    fn cut_starts_on_the_previous_keyframe_fragment() {
        let dir = TestDir::new("mp4-cut-sync");
        let src = dir.path().join("000001.mp4");
        std::fs::write(&src, file(TS, &six_seconds())).unwrap();
        let dst = dir.path().join("clip.mp4");
        // 3.2 s lies in the fourth fragment, which doesn't start on a keyframe: start at 2 s.
        let cut = cut(&src, 3_200, 3_800, &dst).unwrap().unwrap();
        assert_eq!((cut.start_ms, cut.end_ms), (2_000, 4_000));
        let index = assert_well_formed(&std::fs::read(&dst).unwrap());
        assert_eq!(index.fragments.len(), 2);
        assert!(index.fragments[0].first_sync);
    }

    #[test]
    fn one_moment_and_edge_ranges() {
        let dir = TestDir::new("mp4-cut-edges");
        let src = dir.path().join("000001.mp4");
        let bytes = file(TS, &six_seconds());
        std::fs::write(&src, &bytes).unwrap();
        let source = index_bytes(&bytes);
        let dst = dir.path().join("clip.mp4");
        // One moment: the fragment holding it.
        let cut1 = cut(&src, 4_500, 4_500, &dst).unwrap().unwrap();
        assert_eq!((cut1.start_ms, cut1.end_ms), (4_000, 5_000));
        // A moment on a boundary belongs to the fragment starting there.
        assert_eq!(
            plan(&source, 2_000, 2_000).map(|c| (c.start_ms, c.end_ms)),
            Some((2_000, 3_000))
        );
        // The very start and the whole file.
        assert_eq!(plan(&source, 0, 0).map(|c| c.start_ms), Some(0));
        let all = plan(&source, 0, 60_000).unwrap();
        assert_eq!((all.start_ms, all.end_ms), (0, 6_000));
        // Everything but the uuid box.
        assert_eq!(all.bytes, bytes.len() as u64 - 24);
        // Ranges after the end, and a range ending right where the video ends.
        assert_eq!(plan(&source, 6_000, 7_000), None);
        assert_eq!(plan(&source, 5_999, 7_000).map(|c| c.start_ms), Some(5_000));
        assert_eq!(cut(&src, 7_000, 9_000, &dst).unwrap(), None);
        // An end before the start is a moment.
        assert_eq!(plan(&source, 4_500, 1_000).map(|c| c.end_ms), Some(5_000));
        // Nothing to cut from a file without fragments.
        let head = dir.path().join("head.mp4");
        std::fs::write(&head, &bytes[..source.fragments[0].offset as usize]).unwrap();
        assert_eq!(cut(&head, 0, 1_000, &dst).unwrap(), None);
    }

    #[test]
    fn a_range_before_any_keyframe_starts_at_the_first_one() {
        // Two fragments without a keyframe, then normal ones.
        let mut fragments = six_seconds();
        fragments[0] = samples(15, FRAME, 100, false);
        fragments[1] = samples(15, FRAME, 100, false);
        let bytes = file(TS, &fragments);
        let source = index_bytes(&bytes);
        assert_eq!(
            plan(&source, 500, 2_500).map(|c| (c.start_ms, c.end_ms)),
            Some((2_000, 3_000))
        );
        // Nothing playable within the range at all.
        assert_eq!(plan(&source, 0, 1_500), None);
    }

    #[test]
    fn a_cut_of_a_growing_file_takes_its_complete_fragments() {
        let dir = TestDir::new("mp4-cut-growing");
        let src = dir.path().join("000001.mp4");
        let bytes = file(TS, &six_seconds());
        let whole = index_bytes(&bytes);
        let torn = (whole.fragments[4].offset + 50) as usize;
        std::fs::write(&src, &bytes[..torn]).unwrap();
        let dst = dir.path().join("clip.mp4");
        let cut = cut(&src, 2_000, 10_000, &dst).unwrap().unwrap();
        assert_eq!((cut.start_ms, cut.end_ms), (2_000, 4_000));
        assert_eq!(
            assert_well_formed(&std::fs::read(&dst).unwrap())
                .fragments
                .len(),
            2
        );
    }

    #[test]
    fn a_cut_past_its_deadline_leaves_nothing() {
        let dir = TestDir::new("mp4-cut-deadline");
        let src = dir.path().join("000001.mp4");
        std::fs::write(&src, file(TS, &six_seconds())).unwrap();
        let dst = dir.path().join("clip.mp4.tmp");
        let err = cut_until(&src, 0, 5_000, &dst, Some(Instant::now())).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(!dst.exists());
        assert!(cut(&dir.path().join("missing.mp4"), 0, 1, &dst).is_err());
    }

    #[test]
    fn large_box_sizes_and_tfdt_are_read() {
        // A 64-bit size on the mdat and a tfdt on the fragment.
        let mut bytes = [ftyp(), moov(1_000)].concat();
        let at = bytes.len() as u64;
        let pair = fragment(at, 1, &samples(2, 500, 4, true), 9);
        let moof_len = u32::from_be_bytes(pair[..4].try_into().unwrap()) as usize;
        let (moof, mdat) = pair.split_at(moof_len);
        bytes.extend_from_slice(moof);
        let payload = &mdat[8..];
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(&(payload.len() as u64 + 16).to_be_bytes());
        bytes.extend_from_slice(payload);
        let index = index_bytes(&bytes);
        assert_eq!(index.fragments.len(), 1);
        assert_eq!(index.fragments[0].mdat_size, payload.len() as u64 + 16);
        assert_eq!(index.fragments[0].duration, 1_000);

        let mut moof = moof.to_vec();
        // Insert a version 1 tfdt of 7 s into the traf, growing the moof and traf sizes.
        let tfdt = bx(
            b"tfdt",
            &[&[1u8, 0, 0, 0][..], &7_000u64.to_be_bytes()].concat(),
        );
        let traf_at = 8 + 16;
        moof.splice(traf_at + 8..traf_at + 8, tfdt.iter().copied());
        let grow = |buf: &mut Vec<u8>, at: usize| {
            let size = u32::from_be_bytes(buf[at..at + 4].try_into().unwrap()) + tfdt.len() as u32;
            buf[at..at + 4].copy_from_slice(&size.to_be_bytes());
        };
        grow(&mut moof, 0);
        grow(&mut moof, traf_at);
        let parsed = parse_moof(&moof, &Movie::default()).unwrap();
        assert_eq!(parsed.decode_time, Some(7_000));
        assert_eq!(parsed.samples, 2);
    }
}
