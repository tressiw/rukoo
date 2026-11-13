//! A VideoToolbox H.264 encoder: BGRA pixel buffers in, AVCC samples out.
//!
//! Real use requires a hardware encoder (`kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder`),
//! as the Windows path requires one; the `software-encoder` test switch names Apple's software
//! encoder (`com.apple.videotoolbox.videoencoder.h264`) instead, which is what a CI runner has.
//! The session is real-time with no frame reordering, a keyframe interval of 15 frames, and the
//! same bitrate rule as the Windows writer: two bits per pixel a second, at most 8 Mbit/s.
//!
//! The caller drives one frame at a time and waits for its sample, so the samples reach the MP4
//! writer in display order with their own capture times. Frames the writer wants as keyframes
//! (because a fragment is closing) are forced with `kVTEncodeFrameOptionKey_ForceKeyFrame`.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Condvar, Mutex, MutexGuard};

use objc2_core_foundation::{
    kCFBooleanFalse, kCFBooleanTrue, CFArray, CFBoolean, CFDictionary, CFNumber, CFRetained,
    CFString, CFType,
};
use objc2_core_media::{
    kCMSampleAttachmentKey_NotSync, CMBlockBuffer, CMFormatDescription, CMSampleBuffer, CMTime,
    CMVideoCodecType, CMVideoFormatDescriptionGetDimensions,
    CMVideoFormatDescriptionGetH264ParameterSetAtIndex,
};
use objc2_core_video::{CVImageBuffer, CVPixelBuffer};
use objc2_video_toolbox::{
    kVTCompressionPropertyKey_AllowFrameReordering, kVTCompressionPropertyKey_AverageBitRate,
    kVTCompressionPropertyKey_EncoderID, kVTCompressionPropertyKey_ExpectedFrameRate,
    kVTCompressionPropertyKey_MaxKeyFrameInterval, kVTCompressionPropertyKey_ProfileLevel,
    kVTCompressionPropertyKey_RealTime,
    kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
    kVTEncodeFrameOptionKey_ForceKeyFrame, kVTProfileLevel_H264_High_AutoLevel,
    kVTVideoEncoderList_CodecType, kVTVideoEncoderList_DisplayName, kVTVideoEncoderList_EncoderID,
    kVTVideoEncoderList_EncoderName, kVTVideoEncoderList_IsHardwareAccelerated,
    kVTVideoEncoderSpecification_EncoderID,
    kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder, VTCompressionSession,
    VTCopyVideoEncoderList, VTEncodeInfoFlags, VTSession, VTSessionCopyProperty,
    VTSessionSetProperty,
};
use voice_screen_files::caps::EncoderInfo;

use crate::capture::mac::PixelBuffer;
use crate::video::FPS;

/// `avc1`, the codec of the screen's video files.
pub const CODEC_H264: CMVideoCodecType = 0x6176_6331;
/// The id Apple's software H.264 encoder answers to.
pub const SOFTWARE_ENCODER: &str = "com.apple.videotoolbox.videoencoder.h264";

/// The encoder's samples, in display order, and how many frames are still being encoded.
#[derive(Default)]
struct Sink {
    samples: Mutex<EncodedState>,
    ready: Condvar,
}

#[derive(Default)]
struct EncodedState {
    samples: VecDeque<Sample>,
    /// Frames handed to the session, and frames whose callback came back.
    submitted: u64,
    completed: u64,
    /// The first failure a callback reported.
    failure: Option<i32>,
    /// The H.264 parameter sets from the encoder's first format description.
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
    width: u32,
    height: u32,
}

impl Sink {
    fn state(&self) -> MutexGuard<'_, EncodedState> {
        self.samples.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One encoded frame.
#[derive(Debug)]
pub struct Sample {
    /// Whether the sample is a keyframe, from its `NotSync` attachment.
    pub sync: bool,
    /// AVCC: each NAL unit prefixed with its 4-byte big-endian length.
    pub data: Vec<u8>,
}

/// Which encoder a session asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A hardware encoder is required; without one the session fails.
    Hardware,
    /// Apple's software H.264 encoder.
    Software,
}

impl Kind {
    fn specification(self) -> Option<CFRetained<CFDictionary<CFString, CFType>>> {
        match self {
            Kind::Hardware => {
                // SAFETY: the static boolean outlives the dictionary, which retains it.
                let value: &CFBoolean = unsafe { kCFBooleanTrue }?;
                Some(CFDictionary::from_slices(
                    &[unsafe {
                        kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder
                    }],
                    &[as_cf_type(value)],
                ))
            }
            Kind::Software => Some(CFDictionary::from_slices(
                &[unsafe { kVTVideoEncoderSpecification_EncoderID }],
                &[as_cf_type(&*CFString::from_static_str(SOFTWARE_ENCODER))],
            )),
        }
    }
}

/// One compression session, one frame at a time.
pub struct Encoder {
    session: CFRetained<VTCompressionSession>,
    /// The callback's refcon; a boxed value so the pointer stays put when the encoder moves.
    sink: Box<Sink>,
    width: u32,
    height: u32,
    kind: Kind,
    name: String,
    hardware: bool,
}

// SAFETY: the encoder is created and used on the video thread only; VideoToolbox sessions may be
// used from a thread of their own, and the sink is behind a mutex.
unsafe impl Send for Encoder {}

impl Encoder {
    /// A session for frames of `width` x `height`, as `kind` asks.
    pub fn create(width: u32, height: u32, kind: Kind) -> Result<Encoder, String> {
        let (width, height) = (width.max(2) as i32, height.max(2) as i32);
        let sink = Box::new(Sink::default());
        let refcon = &*sink as *const Sink as *mut c_void;
        let specification = kind.specification();
        let mut session: *mut VTCompressionSession = std::ptr::null_mut();
        // SAFETY: the specification is a valid dictionary or absent, the refcon outlives the
        // session (it is the encoder's field), and `session` receives the retained session.
        let status = unsafe {
            VTCompressionSession::create(
                None,
                width,
                height,
                CODEC_H264,
                specification.as_deref().map(opaque),
                None,
                None,
                Some(output_callback),
                refcon,
                NonNull::new(&mut session).expect("a pointer to the session"),
            )
        };
        if status != 0 || session.is_null() {
            return Err(format!("VTCompressionSessionCreate failed: {status}"));
        }
        // SAFETY: the call returns a session with a reference of its own.
        let session = unsafe { CFRetained::from_raw(NonNull::new(session).expect("a session")) };
        let encoder = Encoder {
            session,
            sink,
            width: width as u32,
            height: height as u32,
            kind,
            name: String::new(),
            hardware: false,
        };
        encoder.apply_properties()?;
        // SAFETY: the session is valid; preparing it allocates its encoder.
        let status = unsafe { encoder.session.prepare_to_encode_frames() };
        if status != 0 {
            return Err(format!(
                "VTCompressionSessionPrepareToEncodeFrames failed: {status}"
            ));
        }
        Ok(encoder)
    }

    fn apply_properties(&self) -> Result<(), String> {
        let pixels = u64::from(self.width) * u64::from(self.height);
        let bitrate = (pixels * 2).min(8_000_000) as i32;
        // An encoder that refuses one setting still records, as on Windows.
        let _ = self.set_int(
            unsafe { kVTCompressionPropertyKey_MaxKeyFrameInterval },
            FPS as i32,
        );
        let _ = self.set_bool(unsafe { kVTCompressionPropertyKey_RealTime }, true);
        let _ = self.set_bool(
            unsafe { kVTCompressionPropertyKey_AllowFrameReordering },
            false,
        );
        let _ = self.set_int(unsafe { kVTCompressionPropertyKey_AverageBitRate }, bitrate);
        let _ = self.set_int(
            unsafe { kVTCompressionPropertyKey_ExpectedFrameRate },
            FPS as i32,
        );
        // SAFETY: the static profile string outlives the call.
        let profile: &CFString = unsafe { kVTProfileLevel_H264_High_AutoLevel };
        let _ = self.set(profile_key(), as_cf_type(profile));
        Ok(())
    }

    fn set(&self, key: &CFString, value: &CFType) -> Result<(), String> {
        // SAFETY: the session is valid and the value is a CF object it retains.
        let status =
            unsafe { VTSessionSetProperty(as_vt_session(&self.session), key, Some(value)) };
        if status != 0 {
            return Err(format!("VTSessionSetProperty failed: {status}"));
        }
        Ok(())
    }

    fn set_int(&self, key: &CFString, value: i32) -> Result<(), String> {
        self.set(key, as_cf_type(&*CFNumber::new_i32(value)))
    }

    fn set_bool(&self, key: &CFString, value: bool) -> Result<(), String> {
        // SAFETY: the two boolean singletons are static.
        let flag = unsafe {
            if value {
                kCFBooleanTrue
            } else {
                kCFBooleanFalse
            }
        };
        match flag {
            Some(flag) => self.set(key, as_cf_type(flag)),
            None => Ok(()),
        }
    }

    /// Encodes `picture` at meeting time `at_ms`, forcing a keyframe when `keyframe` is set, and
    /// returns its samples. The samples are AVCC, as `voice_screen_files::fmp4` writes them.
    pub fn encode(
        &mut self,
        picture: &PixelBuffer,
        at_ms: u64,
        keyframe: bool,
    ) -> Result<Vec<Sample>, String> {
        if (picture.width(), picture.height()) != (self.width, self.height) {
            return Err(format!(
                "a {}x{} frame doesn't fit the {}x{} encoder",
                picture.width(),
                picture.height(),
                self.width,
                self.height
            ));
        }
        let submitted = {
            let mut state = self.sink.state();
            state.submitted += 1;
            state.submitted
        };
        // SAFETY: the pixel buffer is a CVImageBuffer, and the time values are valid.
        let (pts, duration) = unsafe {
            (
                CMTime::new((at_ms as i64) * i64::from(FPS), 1_000 * FPS as i32),
                CMTime::new(1_000, 1_000 * FPS as i32),
            )
        };
        let properties = keyframe.then(|| {
            // SAFETY: the static key and the boolean outlive the dictionary.
            let flag: &CFBoolean = unsafe { kCFBooleanTrue }.expect("the true singleton");
            CFDictionary::from_slices(
                &[unsafe { kVTEncodeFrameOptionKey_ForceKeyFrame }],
                &[as_cf_type(flag)],
            )
        });
        let image: &CVImageBuffer =
            unsafe { &*(picture.buffer() as *const CVPixelBuffer as *const CVImageBuffer) };
        // SAFETY: the session is valid; the callback receives the sink through the refcon.
        let status = unsafe {
            self.session.encode_frame(
                image,
                pts,
                duration,
                properties.as_deref().map(opaque),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != 0 {
            return Err(format!("VTCompressionSessionEncodeFrame failed: {status}"));
        }
        // Take this frame's sample before returning, so the writer sees them in order.
        // SAFETY: completing with a non-numeric time flushes every pending frame.
        let status = unsafe { self.session.complete_frames(CMTime::new(0, 0)) };
        if status != 0 {
            return Err(format!(
                "VTCompressionSessionCompleteFrames failed: {status}"
            ));
        }
        let mut state = self.sink.state();
        while state.completed < submitted {
            state = self
                .sink
                .ready
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
        if let Some(status) = state.failure.take() {
            return Err(format!("the encoder failed: {status}"));
        }
        Ok(state.samples.drain(..).collect())
    }

    /// The parameter sets of the first encoded frame, as `avcC` needs them.
    pub fn parameter_sets(&self) -> Option<(Vec<u8>, Vec<u8>)> {
        let state = self.sink.state();
        Some((state.sps.clone()?, state.pps.clone()?))
    }

    /// The encoder's own picture size.
    pub fn size(&self) -> (u32, u32) {
        let state = self.sink.state();
        if state.width > 0 && state.height > 0 {
            (state.width, state.height)
        } else {
            (self.width, self.height)
        }
    }

    /// Asks the session which encoder it loaded, and whether it is hardware. Called after the
    /// first frame, when VideoToolbox has settled on one.
    pub fn identify(&mut self) {
        let id = self
            .copy(unsafe { kVTCompressionPropertyKey_EncoderID })
            .and_then(|value| as_string(&value));
        let hardware = self
            .copy(unsafe { kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder })
            .and_then(|value| as_bool(&value));
        self.hardware = hardware.unwrap_or(match self.kind {
            Kind::Hardware => true,
            Kind::Software => false,
        });
        self.name = id
            .as_deref()
            .and_then(|id| {
                encoder_list()
                    .into_iter()
                    .find(|e| e.id == id)
                    .map(|e| e.name)
            })
            .or_else(|| {
                // No list entry: a name that says what the session was asked for.
                match self.kind {
                    Kind::Hardware => Some("Hardware H.264 encoder".into()),
                    Kind::Software => Some("Apple H.264 (SW)".into()),
                }
            })
            .unwrap_or_else(|| "H.264 encoder".into());
    }

    /// The encoder's name and whether it is hardware.
    pub fn description(&self) -> (&str, bool) {
        (&self.name, self.hardware)
    }

    fn copy(&self, key: &CFString) -> Option<CFRetained<CFType>> {
        let mut out: *mut c_void = std::ptr::null_mut();
        // SAFETY: the session is valid and `out` receives a retained property value.
        let status = unsafe {
            VTSessionCopyProperty(
                as_vt_session(&self.session),
                key,
                None,
                &mut out as *mut *mut c_void as *mut c_void,
            )
        };
        if status != 0 {
            return None;
        }
        let out = NonNull::new(out.cast::<CFType>())?;
        // SAFETY: the property came back with a reference of its own.
        Some(unsafe { CFRetained::from_raw(out) })
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY: invalidating a session we own stops its callbacks before the sink drops.
        unsafe { self.session.invalidate() };
    }
}

/// The callback VideoToolbox runs for every encoded frame. It only reads the sink handed to it
/// as the refcon, which the encoder keeps alive until the session is invalidated.
unsafe extern "C-unwind" fn output_callback(
    refcon: *mut c_void,
    _source: *mut c_void,
    status: i32,
    _flags: VTEncodeInfoFlags,
    sample_buffer: *mut CMSampleBuffer,
) {
    let Some(sink) = NonNull::new(refcon.cast::<Sink>()).map(|p| unsafe { p.as_ref() }) else {
        return;
    };
    if status != 0 || sample_buffer.is_null() {
        let mut state = sink.state();
        state.completed += 1;
        state.failure.get_or_insert(status);
        sink.ready.notify_all();
        return;
    }
    // SAFETY: the callback's sample buffer is valid for the call.
    let sample = unsafe { &*sample_buffer };
    let data = unsafe { sample.data_buffer() }.and_then(|buffer| read_block(&buffer));
    let sync = unsafe { !is_not_sync(sample) };
    if let Some(format) = unsafe { sample.format_description() } {
        let mut state = sink.state();
        let dimensions = unsafe { CMVideoFormatDescriptionGetDimensions(&format) };
        if state.width == 0 {
            state.width = dimensions.width.max(0) as u32;
            state.height = dimensions.height.max(0) as u32;
        }
        if state.sps.is_none() {
            if let Some((sps, pps)) = parameter_sets(&format) {
                state.sps = Some(sps);
                state.pps = Some(pps);
            }
        }
    }
    if let Some(data) = data {
        let mut state = sink.state();
        state.samples.push_back(Sample { sync, data });
    }
    {
        let mut state = sink.state();
        state.completed += 1;
    }
    sink.ready.notify_all();
}

/// A sample buffer's bytes, copied out of the block buffer.
fn read_block(buffer: &CMBlockBuffer) -> Option<Vec<u8>> {
    let len = unsafe { buffer.data_length() };
    if len == 0 {
        return None;
    }
    let mut bytes = vec![0u8; len];
    let destination = NonNull::new(bytes.as_mut_ptr().cast::<c_void>())?;
    // SAFETY: the destination has `len` bytes and the block buffer is at least that long.
    if unsafe { buffer.copy_data_bytes(0, len, destination) } != 0 {
        return None;
    }
    Some(bytes)
}

/// Whether the sample's attachments say it isn't a sync sample.
unsafe fn is_not_sync(sample: &CMSampleBuffer) -> bool {
    // SAFETY: `false` asks for the attachments only when there are any.
    let Some(array) = (unsafe { sample.sample_attachments_array(false) }) else {
        return false;
    };
    if CFArray::count(&array) == 0 {
        return false;
    }
    // SAFETY: the array holds one dictionary per sample.
    let dict = unsafe { CFArray::value_at_index(&array, 0) };
    let Some(dict) = NonNull::new(dict.cast_mut()) else {
        return false;
    };
    // SAFETY: the entry is a CFDictionary.
    let dict: &CFDictionary<CFString, CFBoolean> =
        unsafe { &*dict.as_ptr().cast::<CFDictionary<CFString, CFBoolean>>() };
    // SAFETY: the key is the static attachment key.
    let key: &CFString = unsafe { kCMSampleAttachmentKey_NotSync };
    dict.get(key).is_some_and(|value| value.value())
}

/// The H.264 parameter sets from a format description, as (SPS, PPS).
fn parameter_sets(format: &CMFormatDescription) -> Option<(Vec<u8>, Vec<u8>)> {
    let mut count = 0usize;
    let mut header_length = 0i32;
    // SAFETY: the pointers to the count and header length are valid; the rest is null.
    let status = unsafe {
        CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
            format,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut count,
            &mut header_length,
        )
    };
    if status != 0 || count < 2 || header_length != 4 {
        return None;
    }
    let mut sets = Vec::with_capacity(count);
    for index in 0..count {
        let (mut pointer, mut size) = (std::ptr::null::<u8>(), 0usize);
        // SAFETY: the pointers receive the parameter set's address and length.
        let status = unsafe {
            CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                format,
                index,
                &mut pointer,
                &mut size,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != 0 || pointer.is_null() || size == 0 {
            return None;
        }
        // SAFETY: the format description holds the set for `size` bytes while it lives.
        sets.push(unsafe { std::slice::from_raw_parts(pointer, size) }.to_vec());
    }
    Some((sets[0].clone(), sets[1].clone()))
}

/// One H.264 encoder VideoToolbox lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedEncoder {
    pub name: String,
    pub id: String,
    pub hardware: bool,
}

/// The H.264 encoders `VTCopyVideoEncoderList` reports, hardware first. `IsHardwareAccelerated`
/// decides, or without it the `…ave…` id Apple's hardware encoder answers to.
pub fn encoder_list() -> Vec<ListedEncoder> {
    let mut list: *const CFArray = std::ptr::null();
    // SAFETY: the call fills in the array pointer, which is released below.
    let status =
        unsafe { VTCopyVideoEncoderList(None, NonNull::new(&mut list).expect("a pointer")) };
    let Some(list) = NonNull::new(list.cast_mut()) else {
        return Vec::new();
    };
    if status != 0 {
        return Vec::new();
    }
    // SAFETY: the call returned a retained array.
    let list = unsafe { CFRetained::from_raw(list) };
    let mut encoders = Vec::new();
    for index in 0..CFArray::count(&list) {
        // SAFETY: the index is inside the array, which holds dictionaries.
        let entry = NonNull::new(unsafe { CFArray::value_at_index(&list, index) }.cast_mut());
        let Some(entry) = entry else {
            continue;
        };
        // SAFETY: each list entry is a CFDictionary.
        let entry: &CFDictionary<CFString, CFType> =
            unsafe { &*entry.as_ptr().cast::<CFDictionary<CFString, CFType>>() };
        let codec = entry
            .get(unsafe { kVTVideoEncoderList_CodecType })
            .and_then(|value| as_i32(&value));
        if codec.is_some_and(|codec| codec as u32 != CODEC_H264) {
            continue;
        }
        let id = entry
            .get(unsafe { kVTVideoEncoderList_EncoderID })
            .and_then(|value| as_string(&value))
            .unwrap_or_default();
        let name = entry
            .get(unsafe { kVTVideoEncoderList_EncoderName })
            .or_else(|| entry.get(unsafe { kVTVideoEncoderList_DisplayName }))
            .and_then(|value| as_string(&value))
            .unwrap_or_else(|| id.clone());
        let hardware = entry
            .get(unsafe { kVTVideoEncoderList_IsHardwareAccelerated })
            .and_then(|value| as_bool(&value))
            .unwrap_or_else(|| id.contains(".ave."));
        encoders.push(ListedEncoder { name, id, hardware });
    }
    encoders.sort_by_key(|e| !e.hardware);
    encoders
}

/// Whether a hardware H.264 encoder is installed: the list says so, and a session that requires
/// one starts. The answer is cached: encoders don't appear while the process runs.
pub fn hardware_available() -> bool {
    use std::sync::OnceLock;
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let listed = encoder_list().iter().any(|e| e.hardware);
        if !listed {
            return false;
        }
        // The list is a hint; a session that requires hardware is the proof.
        Encoder::create(320, 180, Kind::Hardware).is_ok()
    })
}

/// The encoders this computer records with, hardware first, as `probe` prints them.
pub fn h264_encoders() -> Vec<EncoderInfo> {
    let listed = encoder_list();
    let mut encoders: Vec<EncoderInfo> = listed
        .iter()
        .map(|e| EncoderInfo {
            name: e.name.clone(),
            hardware: e.hardware,
            vendor: None,
        })
        .collect();
    // A requirable session is the second opinion on hardware; its encoder joins the list when
    // the listing missed it.
    if let Ok(mut session) = Encoder::create(320, 180, Kind::Hardware) {
        session.identify();
        let (name, hardware) = session.description();
        if hardware && !listed.iter().any(|e| e.name == name) {
            encoders.insert(
                0,
                EncoderInfo {
                    name: name.to_string(),
                    hardware: true,
                    vendor: None,
                },
            );
        }
    }
    encoders
}

/// A `CFDictionary` of concrete types as the untyped one VideoToolbox takes.
fn opaque<K: Sized, V: Sized>(dict: &CFDictionary<K, V>) -> &CFDictionary {
    // SAFETY: every CFDictionary is the same Core Foundation object.
    unsafe { &*(dict as *const CFDictionary<K, V> as *const CFDictionary) }
}

/// A Core Foundation object as the untyped `CFType` a property takes.
fn as_cf_type<T: Sized>(value: &T) -> &CFType {
    // SAFETY: only CF objects are handed to this.
    unsafe { &*(value as *const T as *const CFType) }
}

fn as_string(value: &CFType) -> Option<String> {
    // SAFETY: the caller knows the property is a CFString.
    let value: &CFString = unsafe { &*(value as *const CFType as *const CFString) };
    Some(value.to_string())
}

fn as_bool(value: &CFType) -> Option<bool> {
    // SAFETY: the caller knows the property is a CFBoolean.
    let value: &CFBoolean = unsafe { &*(value as *const CFType as *const CFBoolean) };
    Some(value.value())
}

fn as_i32(value: &CFType) -> Option<i32> {
    // SAFETY: the caller knows the property is a CFNumber.
    let value: &CFNumber = unsafe { &*(value as *const CFType as *const CFNumber) };
    value.as_i32()
}

/// A `VTCompressionSession` as the `VTSession` its property calls take.
fn as_vt_session(session: &VTCompressionSession) -> &VTSession {
    // SAFETY: a compression session is a VTSession.
    unsafe { &*(session as *const VTCompressionSession as *const VTSession) }
}

/// `kVTCompressionPropertyKey_ProfileLevel`, read once per session.
fn profile_key() -> &'static CFString {
    // SAFETY: the key is a static string the framework owns.
    unsafe { kVTCompressionPropertyKey_ProfileLevel }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_has_an_h264_encoder() {
        let encoders = encoder_list();
        assert!(!encoders.is_empty(), "VideoToolbox lists no H.264 encoder");
        for encoder in &encoders {
            assert!(!encoder.name.is_empty(), "{encoder:?}");
        }
        // Hardware first: the sort is what `probe` prints.
        let first_software = encoders.iter().position(|e| !e.hardware);
        let last_hardware = encoders.iter().rposition(|e| e.hardware);
        if let (Some(first_software), Some(last_hardware)) = (first_software, last_hardware) {
            assert!(last_hardware < first_software, "{encoders:?}");
        }
        // Apple's software encoder is always there.
        assert!(
            encoders
                .iter()
                .any(|e| e.id == SOFTWARE_ENCODER || e.name.contains("SW")),
            "{encoders:?}"
        );
    }

    /// The software encoder writes a keyframe first, then samples that fit the format: AVCC
    /// with 4-byte lengths, and a keyframe asked for exactly once.
    #[test]
    fn the_software_encoder_produces_samples() {
        let (width, height) = (320u32, 180u32);
        let mut encoder = Encoder::create(width, height, Kind::Software).unwrap();
        encoder.identify();
        assert!(
            !encoder.description().1,
            "the software encoder is hardware?"
        );
        let picture = PixelBuffer::new(width, height).unwrap();
        picture.fill([0x20, 0x60, 0xa0, 0xff]);
        let first = encoder.encode(&picture, 0, true).unwrap();
        assert_eq!(first.len(), 1, "one sample for one frame");
        assert!(first[0].sync, "the forced keyframe is a sync sample");
        let (sps, pps) = encoder.parameter_sets().expect("parameter sets");
        assert_eq!((sps[0] & 0x1f, pps[0] & 0x1f), (7, 8), "SPS and PPS");
        assert_eq!(encoder.size(), (width, height));
        let second = encoder.encode(&picture, 100, false).unwrap();
        assert_eq!(second.len(), 1);
        assert!(!second[0].sync, "the second frame isn't a keyframe");
        // Every NAL unit is length-prefixed, and the lengths add up to the sample.
        let mut at = 0;
        let mut units = 0;
        while at + 4 <= first[0].data.len() {
            let len = u32::from_be_bytes(first[0].data[at..at + 4].try_into().unwrap()) as usize;
            assert!(at + 4 + len <= first[0].data.len(), "a {len}-byte NAL unit");
            at += 4 + len;
            units += 1;
        }
        assert_eq!(at, first[0].data.len());
        assert!(units >= 1);
    }

    /// The hardware encoder on this Mac, when it has one: a 1080p session that reports itself as
    /// hardware. Machines without one skip it.
    #[test]
    fn the_hardware_encoder_encodes_when_there_is_one() {
        if !hardware_available() {
            eprintln!("no hardware H.264 encoder on this computer; skipped");
            return;
        }
        let mut encoder = Encoder::create(1920, 1080, Kind::Hardware).unwrap();
        encoder.identify();
        let (name, hardware) = encoder.description();
        assert!(hardware, "{name}");
        let picture = PixelBuffer::new(1920, 1080).unwrap();
        let samples = encoder.encode(&picture, 0, true).unwrap();
        assert_eq!(samples.len(), 1);
        assert!(samples[0].sync);
    }
}
