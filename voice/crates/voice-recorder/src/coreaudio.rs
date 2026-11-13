//! What the recorder asks Core Audio itself on macOS, beside cpal 0.18.2:
//!
//! - whether the Mac has a microphone at all: cpal hands out a device for `kAudioObjectUnknown`
//!   when it has none, and that device fails in every call and can panic in `to_string`;
//! - whether another app plays sound, which the silence check pairs with a call track of exact
//!   zeros (`silence.rs`);
//! - a process tap of an output device that also has inputs. cpal records such a device's input
//!   instead of tapping it, so the recorder makes the tap and its private aggregate device itself,
//!   the way cpal does for a device without inputs, and opens the aggregate as an input device.
//!
//! Nothing here asks for a permission: creating a tap and an aggregate device doesn't prompt.
//! macOS asks for System Audio Recording when the aggregate's input starts, inside cpal's open.

use std::ffi::{c_void, CStr};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};

use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_core_audio::{
    kAudioAggregateDeviceNameKey, kAudioAggregateDeviceTapAutoStartKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey, kAudioEndPointDeviceIsPrivateKey,
    kAudioHardwareNoError, kAudioHardwarePropertyDefaultInputDevice,
    kAudioHardwarePropertyProcessObjectList, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioObjectUnknown,
    kAudioProcessPropertyIsRunningOutput, kAudioProcessPropertyPID,
    kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey, AudioHardwareCreateAggregateDevice,
    AudioHardwareCreateProcessTap, AudioHardwareDestroyAggregateDevice,
    AudioHardwareDestroyProcessTap, AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize,
    AudioObjectID, AudioObjectPropertyAddress, AudioObjectPropertySelector, CATapDescription,
    CATapMuteBehavior,
};
use objc2_core_foundation::{
    kCFAllocatorDefault, kCFTypeArrayCallBacks, kCFTypeDictionaryKeyCallBacks,
    kCFTypeDictionaryValueCallBacks, CFArray, CFDictionary, CFMutableDictionary, CFRetained,
    CFString,
};
use objc2_foundation::{NSArray, NSNumber, NSString};

fn address(selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress {
        mSelector: selector,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    }
}

/// Reads one fixed-size property, or `None` when the HAL doesn't answer.
fn read<T: Copy + Default>(
    object: AudioObjectID,
    selector: AudioObjectPropertySelector,
) -> Option<T> {
    let address = address(selector);
    let mut value = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    // SAFETY: the address is valid and the HAL writes at most `size` bytes into `value`, whose
    // type is the property's: an object id, a pid or a u32 flag for the selectors used here.
    let status = unsafe {
        AudioObjectGetPropertyData(
            object,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut value).cast(),
        )
    };
    (status == kAudioHardwareNoError && size as usize == std::mem::size_of::<T>()).then_some(value)
}

/// Whether the Mac has no default input device: Core Audio answers `kAudioObjectUnknown`, which
/// cpal 0.18.2 still turns into a device. A HAL that doesn't answer leaves the decision to cpal.
pub fn no_microphone() -> bool {
    missing(read::<AudioObjectID>(
        kAudioObjectSystemObject as AudioObjectID,
        kAudioHardwarePropertyDefaultInputDevice,
    ))
}

/// Whether Core Audio's answer for the default input means "this Mac has no input device".
fn missing(default_input: Option<AudioObjectID>) -> bool {
    default_input == Some(kAudioObjectUnknown)
}

/// Whether a process other than this one is playing sound on some output device right now, from
/// the process list Core Audio keeps (macOS 14.2 and later). `None` when it can't be read.
///
/// It can't say what the sound is: an app that plays exact digital silence, as a call app can
/// between words, counts as playing.
pub fn others_playing() -> Option<bool> {
    let system = kAudioObjectSystemObject as AudioObjectID;
    let address = address(kAudioHardwarePropertyProcessObjectList);
    let mut size = 0u32;
    // SAFETY: the HAL writes the list's size in bytes into `size`.
    let status = unsafe {
        AudioObjectGetPropertyDataSize(
            system,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
        )
    };
    if status != kAudioHardwareNoError {
        return None;
    }
    let mut processes =
        vec![0 as AudioObjectID; size as usize / std::mem::size_of::<AudioObjectID>()];
    if processes.is_empty() {
        return Some(false);
    }
    let mut size = (processes.len() * std::mem::size_of::<AudioObjectID>()) as u32;
    // SAFETY: `processes` has room for `size` bytes of object ids, and the HAL writes no more.
    let status = unsafe {
        AudioObjectGetPropertyData(
            system,
            NonNull::from(&address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new(processes.as_mut_ptr()).unwrap().cast(),
        )
    };
    if status != kAudioHardwareNoError {
        return None;
    }
    processes.truncate(size as usize / std::mem::size_of::<AudioObjectID>());
    let me = std::process::id() as i32;
    Some(processes.into_iter().any(|process| {
        read::<i32>(process, kAudioProcessPropertyPID) != Some(me)
            && read::<u32>(process, kAudioProcessPropertyIsRunningOutput).is_some_and(|on| on != 0)
    }))
}

static TAPS: AtomicU32 = AtomicU32::new(0);

/// A process tap of one output device and the private aggregate device that turns it into an
/// input, destroyed on drop. A hard kill leaves them to the audio server, which drops a private
/// aggregate with the process that made it.
pub struct Tap {
    tap: AudioObjectID,
    aggregate: AudioObjectID,
    /// The aggregate's UID, by which cpal finds it among the devices.
    pub uid: String,
}

impl Tap {
    /// Taps everything `device_uid` plays, from every process, without muting it. These are
    /// cpal's own settings for a device without inputs (`host/coreaudio/macos/loopback.rs`), so
    /// the call track sounds the same whichever path opened it.
    pub fn of_output(device_uid: &str) -> Result<Tap, String> {
        let pid = std::process::id();
        let n = TAPS.fetch_add(1, Ordering::Relaxed);
        let processes: Retained<NSArray<NSNumber>> = NSArray::new();
        let uid = NSString::from_str(device_uid);
        // SAFETY: a freshly allocated description, initialised once with valid arguments, then
        // given plain values through its setters.
        let description = unsafe {
            let description = CATapDescription::initWithProcesses_andDeviceUID_withStream(
                CATapDescription::alloc(),
                &processes,
                &uid,
                0,
            );
            description.setMuteBehavior(CATapMuteBehavior::Unmuted);
            description.setName(&NSString::from_str(&format!("Rukoo Voice call {pid}.{n}")));
            description.setPrivate(true);
            // With an empty list, exclusive means "every process".
            description.setExclusive(true);
            description
        };
        let mut tap: AudioObjectID = kAudioObjectUnknown;
        // SAFETY: the description is valid and the HAL writes the new tap's id into `tap`.
        let status = unsafe { AudioHardwareCreateProcessTap(Some(&description), &mut tap) };
        if status != kAudioHardwareNoError {
            return Err(format!(
                "macOS refused the process tap (OSStatus {status})."
            ));
        }
        // SAFETY: the description's UUID is set when the tap is created.
        let tap_uid = unsafe { description.UUID().UUIDString() };
        let aggregate_uid = format!("nl.bvdm.rukoo.voice.call.{pid}.{n}");
        let properties = aggregate_properties(
            &tap_uid,
            &aggregate_uid,
            &format!("Rukoo Voice call {pid}.{n}"),
        );
        let mut aggregate: AudioObjectID = kAudioObjectUnknown;
        // SAFETY: the dictionary holds the keys Core Audio documents for an aggregate device, and
        // the HAL writes the new device's id into `aggregate`.
        let status = unsafe {
            AudioHardwareCreateAggregateDevice(&properties, NonNull::from(&mut aggregate))
        };
        if status != kAudioHardwareNoError {
            // SAFETY: the tap was created above and nothing else holds it.
            unsafe { AudioHardwareDestroyProcessTap(tap) };
            return Err(format!(
                "macOS refused the device for the process tap (OSStatus {status})."
            ));
        }
        Ok(Tap {
            tap,
            aggregate,
            uid: aggregate_uid,
        })
    }
}

impl Drop for Tap {
    fn drop(&mut self) {
        // SAFETY: both objects were created by `of_output` and are destroyed once. The stream on
        // the aggregate is dropped before the tap (see `source.rs`).
        unsafe {
            AudioHardwareDestroyAggregateDevice(self.aggregate);
            AudioHardwareDestroyProcessTap(self.tap);
        }
    }
}

fn cf(key: &'static CStr) -> CFRetained<CFString> {
    // SAFETY: the keys are NUL-terminated ASCII constants.
    unsafe { CFString::with_c_string(kCFAllocatorDefault, key.as_ptr(), 0x0800_0100) }
        .expect("a Core Audio key is valid UTF-8")
}

/// `{ name, uid, taps: [{ uid: tap, drift: yes }], tapautostart: yes, private: yes }`, as cpal
/// builds it.
fn aggregate_properties(tap_uid: &NSString, uid: &str, name: &str) -> CFRetained<CFDictionary> {
    let yes = NSNumber::new_bool(true);
    let set = |dict: &CFMutableDictionary, key: &CFString, value: *const c_void| {
        // SAFETY: both are Core Foundation objects (NSString and NSNumber are toll-free bridged)
        // that the dictionary retains.
        unsafe {
            CFMutableDictionary::set_value(
                Some(dict),
                key as *const CFString as *const c_void,
                value,
            )
        }
    };
    // SAFETY: Core Foundation's own allocator and type callbacks.
    let tap = unsafe {
        CFMutableDictionary::new(
            kCFAllocatorDefault,
            2,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
    }
    .expect("a dictionary");
    set(
        &tap,
        &cf(kAudioSubTapUIDKey),
        tap_uid as *const NSString as *const c_void,
    );
    set(
        &tap,
        &cf(kAudioSubTapDriftCompensationKey),
        &*yes as *const NSNumber as *const c_void,
    );
    let taps = [CFRetained::as_ptr(&tap).as_ptr() as *const c_void];
    // SAFETY: one valid dictionary, retained by the array.
    let taps = unsafe {
        CFArray::new(
            kCFAllocatorDefault,
            taps.as_ptr() as *mut *const c_void,
            1,
            &kCFTypeArrayCallBacks,
        )
    }
    .expect("an array");
    // SAFETY: as above.
    let dict = unsafe {
        CFMutableDictionary::new(
            kCFAllocatorDefault,
            5,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        )
    }
    .expect("a dictionary");
    let name = CFString::from_str(name);
    let uid = CFString::from_str(uid);
    set(
        &dict,
        &cf(kAudioAggregateDeviceNameKey),
        &*name as *const CFString as *const c_void,
    );
    set(
        &dict,
        &cf(kAudioAggregateDeviceUIDKey),
        &*uid as *const CFString as *const c_void,
    );
    set(
        &dict,
        &cf(kAudioAggregateDeviceTapListKey),
        &*taps as *const CFArray as *const c_void,
    );
    set(
        &dict,
        &cf(kAudioAggregateDeviceTapAutoStartKey),
        &*yes as *const NSNumber as *const c_void,
    );
    set(
        &dict,
        &cf(kAudioEndPointDeviceIsPrivateKey),
        &*yes as *const NSNumber as *const c_void,
    );
    // SAFETY: a mutable dictionary is a dictionary.
    unsafe { CFRetained::cast_unchecked::<CFDictionary>(dict) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_audio_device_is_no_microphone() {
        assert!(missing(Some(0)), "kAudioObjectUnknown");
        assert!(!missing(Some(1)));
        assert!(!missing(Some(0x4E)));
        assert!(
            !missing(None),
            "a HAL that can't answer is cpal's own `None`, not this check"
        );
    }

    /// The Mac mini this port was built on has no input device, the case the check exists for.
    /// The read opens nothing and asks for no permission.
    #[test]
    #[ignore = "needs a Mac without an input device"]
    fn this_mac_reports_no_microphone() {
        assert!(no_microphone());
    }

    /// Reads Core Audio's state without opening a device or asking for anything.
    #[test]
    fn reads_the_process_list_without_opening_anything() {
        // This test process plays nothing; whether another app does depends on the Mac.
        assert!(others_playing().is_some(), "the process list didn't answer");
        let _ = no_microphone();
    }

    /// Builds the aggregate's description without creating anything.
    #[test]
    fn describes_a_private_aggregate_with_one_tap() {
        let dict = aggregate_properties(&NSString::from_str("tap-uid"), "agg-uid", "agg");
        assert_eq!(CFDictionary::count(&dict), 5);
    }

    /// Creates a real tap and aggregate on the default output, then destroys them. It opens no
    /// input, so macOS shouldn't prompt, but it touches the person's audio setup: run it at a Mac
    /// you sit at.
    #[test]
    #[ignore = "creates a process tap on this Mac's default output"]
    fn creates_and_destroys_a_tap() {
        use cpal::traits::{DeviceTrait, HostTrait};
        let host = cpal::default_host();
        let output = host.default_output_device().expect("an output device");
        let id = output.id().expect("its uid").to_string();
        let uid = id.strip_prefix("coreaudio:").expect("a Core Audio id");
        let tap = Tap::of_output(uid).expect("a tap");
        let found = host.devices().unwrap().any(|d| {
            d.id()
                .is_ok_and(|i| i.to_string() == format!("coreaudio:{}", tap.uid))
        });
        assert!(
            found,
            "the private aggregate isn't among this process's devices"
        );
    }
}
