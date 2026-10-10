//! CoreAudio process-tap capture (macOS 14.2+) — the preferred "them"
//! channel backend. A `CATapDescription` global tap (excluding our own
//! process, parity with `excludes_current_process_audio`) feeds a
//! private tap-only aggregate device read via `AudioDeviceIOProc`.
//! Consent is `NSAudioCaptureUsageDescription` — system-audio recording
//! only — so speaker capture lights no "Currently Sharing" screen panel
//! and never asks for screen-recording permission. ScreenCaptureKit
//! stays the fallback in `macos.rs`.

use std::ffi::{c_void, CStr};
use std::ptr::NonNull;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{SyncSender, TrySendError};

use anyhow::Result;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, AnyThread, ClassType};
use objc2_core_audio::{
    kAudioAggregateDeviceIsPrivateKey, kAudioAggregateDeviceIsStackedKey,
    kAudioAggregateDeviceNameKey, kAudioAggregateDeviceTapAutoStartKey,
    kAudioAggregateDeviceTapListKey, kAudioAggregateDeviceUIDKey,
    kAudioHardwarePropertyTranslatePIDToProcessObject, kAudioObjectPropertyElementMain,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject, kAudioObjectUnknown,
    kAudioSubTapDriftCompensationKey, kAudioSubTapUIDKey, kAudioTapPropertyFormat,
    AudioDeviceCreateIOProcID, AudioDeviceDestroyIOProcID, AudioDeviceIOProcID, AudioDeviceStart,
    AudioDeviceStop, AudioHardwareCreateAggregateDevice, AudioHardwareCreateProcessTap,
    AudioHardwareDestroyAggregateDevice, AudioHardwareDestroyProcessTap,
    AudioObjectGetPropertyData, AudioObjectID, AudioObjectPropertyAddress, CATapDescription,
    CATapMuteBehavior,
};
use objc2_core_audio_types::{
    kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved, kAudioFormatLinearPCM,
    AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp,
};
use objc2_core_foundation::CFDictionary;
use objc2_foundation::{
    NSArray, NSDictionary, NSNumber, NSOperatingSystemVersion, NSProcessInfo, NSString, NSUUID,
};

use super::super::{warn_unsupported, RawChunk};
use super::{decode_pcm, AudioBufferBytes};

/// Guard against a garbage `mNumberBuffers` before trusting the trailing
/// `AudioBuffer` array — the HAL is expected to emit 1-2 buffers, not
/// hundreds.
const MAX_TAP_BUFFERS: usize = 16;

/// Live process-tap capture. `stop()` is idempotent so the source and
/// `Drop` can both call it; teardown order is load-bearing (IO first,
/// proc id second, client box, aggregate, tap).
pub(super) struct TapCapture {
    tap_id: AudioObjectID,
    device_id: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    client: *mut c_void,
    started: bool,
}

// SAFETY: the HAL only invokes `io_proc` between `AudioDeviceStart` and
// `AudioDeviceStop`/`AudioDeviceDestroyIOProcID` for a given proc id; the
// client box is freed strictly after both complete in `stop()`. The
// AudioObjectIDs are plain `u32` handles.
unsafe impl Send for TapCapture {}

struct TapClientData {
    tx: SyncSender<RawChunk>,
    warned: AtomicBool,
    sample_rate: u32,
    channels: usize,
    bits: usize,
    is_float: bool,
    non_interleaved: bool,
}

/// PCM format decoded out of the tap's ASBD: `(rate, channels, bits,
/// is_float, non_interleaved)`.
type TapFormat = (u32, usize, usize, bool, bool);

impl TapCapture {
    /// Build and start a tap-only aggregate device. On any failure the
    /// partially built objects are destroyed before the error returns.
    pub(super) fn start(tx: SyncSender<RawChunk>) -> Result<Self> {
        if !process_tap_supported() {
            anyhow::bail!("CoreAudio process taps require macOS 14.2+");
        }
        let mut capture = Self {
            tap_id: kAudioObjectUnknown,
            device_id: kAudioObjectUnknown,
            proc_id: None,
            client: std::ptr::null_mut(),
            started: false,
        };
        if let Err(error) = capture.create(tx) {
            capture.stop();
            return Err(error);
        }
        Ok(capture)
    }

    fn create(&mut self, tx: SyncSender<RawChunk>) -> Result<()> {
        let process_object = own_process_object()
            .ok_or_else(|| anyhow::anyhow!("could not resolve own CoreAudio process object"))?;
        let exclusions: Vec<Retained<NSNumber>> = [process_object]
            .iter()
            .map(|&id| NSNumber::new_u32(id))
            .collect();
        let exclude = NSArray::<NSNumber>::from_retained_slice(&exclusions);
        let description = unsafe {
            CATapDescription::initStereoGlobalTapButExcludeProcesses(
                CATapDescription::alloc(),
                &exclude,
            )
        };
        unsafe {
            description.setName(&NSString::from_str("marvis speaker tap"));
            description.setPrivate(true);
            description.setMuteBehavior(CATapMuteBehavior::Unmuted);
        }
        let tap_uuid: Retained<NSUUID> = unsafe { msg_send![NSUUID::class(), UUID] };
        unsafe { description.setUUID(&tap_uuid) };

        let mut tap_id = kAudioObjectUnknown;
        let status = unsafe { AudioHardwareCreateProcessTap(Some(&*description), &mut tap_id) };
        if status != 0 || tap_id == kAudioObjectUnknown {
            anyhow::bail!("process tap creation failed (OSStatus {status})");
        }
        self.tap_id = tap_id;

        let flag_on = NSNumber::new_bool(true);
        let flag_off = NSNumber::new_bool(false);
        let tap_uid = NSString::from_str(&format!("{tap_uuid}"));
        let tap_entry = NSDictionary::<NSString, AnyObject>::from_slices(
            &[
                &*key(kAudioSubTapUIDKey),
                &*key(kAudioSubTapDriftCompensationKey),
            ],
            &[as_object(&tap_uid), as_object(&flag_on)],
        );
        let tap_list =
            NSArray::<NSDictionary<NSString, AnyObject>>::from_retained_slice(&[tap_entry]);
        let agg_uid = NSString::from_str(&format!("marvis-speaker-tap-{tap_uuid}"));
        let agg_name = NSString::from_str("marvis speaker tap");
        let aggregate = NSDictionary::<NSString, AnyObject>::from_slices(
            &[
                &*key(kAudioAggregateDeviceNameKey),
                &*key(kAudioAggregateDeviceUIDKey),
                &*key(kAudioAggregateDeviceIsPrivateKey),
                &*key(kAudioAggregateDeviceIsStackedKey),
                &*key(kAudioAggregateDeviceTapAutoStartKey),
                &*key(kAudioAggregateDeviceTapListKey),
            ],
            &[
                as_object(&agg_name),
                as_object(&agg_uid),
                as_object(&flag_on),
                as_object(&flag_off),
                as_object(&flag_on),
                as_object(&tap_list),
            ],
        );
        // SAFETY: NSDictionary is toll-free bridged to CFDictionary — the
        // object pointer is valid for the duration of the call.
        let cf_dict = unsafe {
            &*(&*aggregate as *const NSDictionary<NSString, AnyObject>).cast::<CFDictionary>()
        };
        let mut device_id = kAudioObjectUnknown;
        let status =
            unsafe { AudioHardwareCreateAggregateDevice(cf_dict, NonNull::from(&mut device_id)) };
        if status != 0 || device_id == kAudioObjectUnknown {
            anyhow::bail!("tap aggregate device creation failed (OSStatus {status})");
        }
        self.device_id = device_id;

        let (sample_rate, channels, bits, is_float, non_interleaved) = tap_format(tap_id)
            .ok_or_else(|| anyhow::anyhow!("process tap format is unreadable or non-PCM"))?;
        let client = Box::into_raw(Box::new(TapClientData {
            tx,
            warned: AtomicBool::new(false),
            sample_rate,
            channels,
            bits,
            is_float,
            non_interleaved,
        }))
        .cast::<c_void>();
        self.client = client;

        let mut proc_id: AudioDeviceIOProcID = None;
        let status = unsafe {
            AudioDeviceCreateIOProcID(
                device_id,
                Some(io_proc),
                client,
                NonNull::from(&mut proc_id),
            )
        };
        if status != 0 || proc_id.is_none() {
            anyhow::bail!("tap IOProc registration failed (OSStatus {status})");
        }
        self.proc_id = proc_id;

        // AudioDeviceStart is where the "system audio recording" consent
        // prompt can appear — callers must keep this off the async
        // executor and the main thread (AGENTS rule 15).
        let status = unsafe { AudioDeviceStart(device_id, proc_id) };
        if status != 0 {
            anyhow::bail!("tap device start failed (OSStatus {status})");
        }
        self.started = true;
        Ok(())
    }

    /// Reverse-order teardown; every step is conditional so this is safe
    /// on a partially built capture and idempotent on a live one.
    pub(super) fn stop(&mut self) {
        if self.started {
            let status = unsafe { AudioDeviceStop(self.device_id, self.proc_id) };
            if status != 0 {
                log::warn!("tap device stop returned OSStatus {status}");
            }
            self.started = false;
        }
        if let Some(proc_id) = self.proc_id.take() {
            unsafe { AudioDeviceDestroyIOProcID(self.device_id, Some(proc_id)) };
        }
        if !self.client.is_null() {
            drop(unsafe { Box::from_raw(self.client.cast::<TapClientData>()) });
            self.client = std::ptr::null_mut();
        }
        if self.device_id != kAudioObjectUnknown {
            unsafe { AudioHardwareDestroyAggregateDevice(self.device_id) };
            self.device_id = kAudioObjectUnknown;
        }
        if self.tap_id != kAudioObjectUnknown {
            unsafe { AudioHardwareDestroyProcessTap(self.tap_id) };
            self.tap_id = kAudioObjectUnknown;
        }
    }
}

impl Drop for TapCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The HAL realtime callback — no locks, no allocation beyond the
/// bounded `Vec`s, `try_send` only so a stalled consumer never stalls
/// the audio thread.
unsafe extern "C-unwind" fn io_proc(
    _device: AudioObjectID,
    _now: NonNull<AudioTimeStamp>,
    input: NonNull<AudioBufferList>,
    _input_time: NonNull<AudioTimeStamp>,
    _output: NonNull<AudioBufferList>,
    _output_time: NonNull<AudioTimeStamp>,
    client_data: *mut c_void,
) -> i32 {
    if client_data.is_null() {
        return 0;
    }
    let client = unsafe { &*(client_data as *const TapClientData) };
    let list = unsafe { input.as_ref() };
    let count = (list.mNumberBuffers as usize).min(MAX_TAP_BUFFERS);
    let raw = unsafe { std::slice::from_raw_parts(list.mBuffers.as_ptr(), count) };
    let buffers: Vec<AudioBufferBytes> = raw
        .iter()
        .filter(|buffer| !buffer.mData.is_null() && buffer.mDataByteSize > 0)
        .map(|buffer| AudioBufferBytes {
            data: unsafe {
                std::slice::from_raw_parts(buffer.mData as *const u8, buffer.mDataByteSize as usize)
            },
            channels: buffer.mNumberChannels as usize,
        })
        .collect();
    let Some(samples) = decode_pcm(
        &buffers,
        client.bits,
        client.is_float,
        client.non_interleaved,
        client.channels,
    ) else {
        warn_unsupported(&client.warned, "tap buffer layout or PCM data was rejected");
        return 0;
    };
    match client
        .tx
        .try_send((samples, client.sample_rate, client.channels as u16))
    {
        Ok(()) | Err(TrySendError::Disconnected(_)) => {}
        Err(TrySendError::Full(_)) => log::debug!("dropping stale tap audio chunk"),
    }
    0
}

/// `kAudioObjectSystemObject`'s PID→process-object translation — how we
/// exclude our own audio from the global tap (the tap description takes
/// AudioObjectIDs, not PIDs).
fn own_process_object() -> Option<AudioObjectID> {
    let mut address = AudioObjectPropertyAddress {
        mSelector: kAudioHardwarePropertyTranslatePIDToProcessObject,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };
    let pid = std::process::id() as i32;
    let mut id = kAudioObjectUnknown;
    let mut size = std::mem::size_of::<AudioObjectID>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&mut address),
            std::mem::size_of::<i32>() as u32,
            &pid as *const i32 as *const c_void,
            NonNull::from(&mut size),
            NonNull::from(&mut id).cast(),
        )
    };
    (status == 0 && id != kAudioObjectUnknown).then_some(id)
}

/// The tap's `kAudioTapPropertyFormat` ASBD, decoded into the tuple
/// `io_proc` needs. `None` means unreadable or non-PCM — startup fails
/// so the caller can fall back to ScreenCaptureKit.
fn tap_format(tap_id: AudioObjectID) -> Option<TapFormat> {
    let mut address = AudioObjectPropertyAddress {
        mSelector: kAudioTapPropertyFormat,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain,
    };
    let mut asbd = std::mem::MaybeUninit::<AudioStreamBasicDescription>::uninit();
    let mut size = std::mem::size_of::<AudioStreamBasicDescription>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            tap_id,
            NonNull::from(&mut address),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut asbd).cast(),
        )
    };
    if status != 0 {
        return None;
    }
    let asbd = unsafe { asbd.assume_init() };
    (asbd.mFormatID == kAudioFormatLinearPCM && asbd.mBitsPerChannel > 0).then_some((
        asbd.mSampleRate.round() as u32,
        asbd.mChannelsPerFrame as usize,
        asbd.mBitsPerChannel as usize,
        asbd.mFormatFlags & kAudioFormatFlagIsFloat != 0,
        asbd.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0,
    ))
}

fn process_tap_supported() -> bool {
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion {
        majorVersion: 14,
        minorVersion: 2,
        patchVersion: 0,
    })
}

/// `NSString` keys for the aggregate/sub-tap dictionaries — the `&CStr`
/// constants the generated bindings expose are the same bytes.
fn key(name: &'static CStr) -> Retained<NSString> {
    NSString::from_str(name.to_str().expect("CoreAudio keys are ASCII"))
}

/// `&AnyObject` view of a retained Foundation object for dictionary
/// values. Takes `&Retained<T>` and derefs the wrapper — passing the
/// wrapper slot itself (rather than the object pointer inside it) hands
/// CoreFoundation a garbage "object" whose first qword is the real
/// pointer; it then treats that pointer as an isa and faults realizing
/// the class (`__NSDictionaryI_new` → `objc_lookUpImpOrForward`).
fn as_object<T>(value: &Retained<T>) -> &AnyObject {
    unsafe { &*((&**value) as *const T).cast::<AnyObject>() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for the `__NSDictionaryI_new` crash: the helper
    /// must return the object pointer INSIDE the `Retained`, not the
    /// address of the wrapper slot — the dictionary messages its values
    /// and a wrapper pointer reads as a bogus isa.
    #[test]
    fn as_object_returns_the_wrapped_object_pointer() {
        let string = NSString::from_str("probe");
        let expected = &*string as *const NSString as usize;
        let object = as_object(&string) as *const AnyObject as usize;
        assert_eq!(object, expected);
    }

    /// A muted tap would silence the user's own playback while we record
    /// — the tap must stay unmuted or Listen makes the call inaudible.
    #[test]
    fn tap_stays_unmuted() {
        let source = include_str!("tap.rs");
        assert!(source.contains("setMuteBehavior(CATapMuteBehavior::Unmuted)"));
    }

    /// The exclusion list keeps Marvis's own output out of the mix
    /// (parity with `excludes_current_process_audio` on the SCK path) —
    /// it must resolve OUR process object, not an arbitrary process.
    #[test]
    fn tap_excludes_own_process() {
        let source = include_str!("tap.rs");
        for needle in [
            "kAudioHardwarePropertyTranslatePIDToProcessObject",
            "initStereoGlobalTapButExcludeProcesses",
        ] {
            assert!(source.contains(needle), "tap must keep {needle}");
        }
    }

    /// Teardown order is load-bearing: IO stops before the proc id is
    /// destroyed, the client box frees before the aggregate, the
    /// aggregate before the tap — any other order hands the HAL a
    /// dangling resource.
    #[test]
    fn teardown_order_is_io_then_device_then_tap() {
        let source = include_str!("tap.rs");
        let body = source
            .split("fn stop(&mut self)")
            .nth(1)
            .expect("stop body not found");
        let stop = body
            .find("AudioDeviceStop")
            .expect("AudioDeviceStop missing");
        let destroy_proc = body
            .find("AudioDeviceDestroyIOProcID")
            .expect("AudioDeviceDestroyIOProcID missing");
        let destroy_agg = body
            .find("AudioHardwareDestroyAggregateDevice")
            .expect("AudioHardwareDestroyAggregateDevice missing");
        let destroy_tap = body
            .find("AudioHardwareDestroyProcessTap")
            .expect("AudioHardwareDestroyProcessTap missing");
        assert!(stop < destroy_proc && destroy_proc < destroy_agg && destroy_agg < destroy_tap);
    }

    /// The aggregate must be tap-only — a `kAudioAggregateDeviceMainSubDeviceKey`
    /// would route the user's real output through our device and change
    /// what they hear.
    #[test]
    fn aggregate_stays_tap_only() {
        let source = include_str!("tap.rs");
        let non_test = source.split("#[cfg(test)]").next().unwrap();
        assert!(!non_test.contains("MainSubDeviceKey"));
    }
}
