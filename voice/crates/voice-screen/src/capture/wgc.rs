//! Windows.Graphics.Capture of one monitor. The frame pool is free-threaded, so frames arrive on
//! a thread-pool thread without a message loop; each is copied into a texture the process owns
//! and the capture's buffer goes back at once. The cursor isn't captured, and Windows' yellow
//! capture border stays (spec 1.1, question 6).

use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};

use voice_screen_files::monitors::MonitorInfo;
use windows::core::{factory, h, IInspectable, Interface};
use windows::Foundation::Metadata::ApiInformation;
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureDirtyRegionMode,
    GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_BOX, D3D11_TEXTURE2D_DESC};
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use super::d3d::{is_device_lost, D3D};
use super::monitors_win;
use super::CapturedFrame;
use crate::clockwin;
use crate::platform::{PlatformError, Source};
use crate::record::Ev;
use crate::video::size::even;

/// 15 fps at most: `MinUpdateInterval` where Windows has it (24H2), else frames closer than this
/// are dropped here.
pub const MIN_INTERVAL_100NS: i64 = 660_000;

/// Which of the newer session properties this Windows has. Setting one that's missing makes
/// capture fail on older builds, so each is checked first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    pub cursor: bool,
    pub min_update_interval: bool,
    pub dirty_regions: bool,
}

pub fn features() -> Features {
    let session = h!("Windows.Graphics.Capture.GraphicsCaptureSession");
    let present = |name| ApiInformation::IsPropertyPresent(session, name).unwrap_or(false);
    Features {
        cursor: present(h!("IsCursorCaptureEnabled")),
        min_update_interval: present(h!("MinUpdateInterval")),
        dirty_regions: present(h!("DirtyRegionMode")),
    }
}

/// Whether this Windows can capture at all.
pub fn supported() -> bool {
    GraphicsCaptureSession::IsSupported().unwrap_or(false)
}

pub struct WgcSource {
    d3d: D3D,
    to_main: SyncSender<Ev<super::Picture>>,
    features: Features,
    live: Option<Live>,
}

struct Live {
    item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    frame_token: i64,
    closed_token: i64,
    handler: Arc<Mutex<Handler>>,
}

pub fn new(d3d: D3D, to_main: SyncSender<Ev<super::Picture>>) -> WgcSource {
    WgcSource {
        d3d,
        to_main,
        features: features(),
        live: None,
    }
}

impl Source for WgcSource {
    fn start(&mut self, monitor: &MonitorInfo, generation: u64) -> Result<(), PlatformError> {
        self.stop();
        let hmon = monitors_win::hmonitor(&monitor.id).ok_or_else(|| {
            windows::core::Error::new(
                windows::core::HRESULT(0x80070490u32 as i32),
                "the monitor isn't there",
            )
        })?;
        let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(hmon)? };
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.d3d.winrt,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        if self.features.cursor {
            session.SetIsCursorCaptureEnabled(false)?;
        }
        if self.features.min_update_interval {
            session.SetMinUpdateInterval(TimeSpan {
                Duration: MIN_INTERVAL_100NS,
            })?;
        }
        if self.features.dirty_regions {
            // ReportOnly: the whole frame is drawn, and its dirty regions are reported.
            session.SetDirtyRegionMode(GraphicsCaptureDirtyRegionMode::ReportOnly)?;
        }
        let handler = Arc::new(Mutex::new(Handler {
            d3d: self.d3d.clone(),
            to_main: self.to_main.clone(),
            generation,
            size,
            last_boot: 0,
            throttle: !self.features.min_update_interval,
            dirty_regions: self.features.dirty_regions,
            carried: 0.0,
            lost: false,
            stopped: false,
        }));
        let on_frame = handler.clone();
        let frame_token = pool.FrameArrived(&TypedEventHandler::<
            Direct3D11CaptureFramePool,
            IInspectable,
        >::new(move |pool, _| {
            let pool = pool.ok()?;
            let mut handler = on_frame.lock().unwrap_or_else(|e| e.into_inner());
            handler.on_frame(pool)
        }))?;
        let to_main = self.to_main.clone();
        let closed_token = item.Closed(
            &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                let _ = to_main.try_send(Ev::CaptureClosed(generation));
                Ok(())
            }),
        )?;
        session.StartCapture()?;
        self.live = Some(Live {
            item,
            pool,
            session,
            frame_token,
            closed_token,
            handler,
        });
        Ok(())
    }

    fn stop(&mut self) {
        if let Some(live) = self.live.take() {
            live.handler
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .stopped = true;
            let _ = live.pool.RemoveFrameArrived(live.frame_token);
            let _ = live.item.RemoveClosed(live.closed_token);
            let _ = live.session.Close();
            let _ = live.pool.Close();
        }
    }
}

impl Drop for WgcSource {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Handler {
    d3d: D3D,
    to_main: SyncSender<Ev<super::Picture>>,
    generation: u64,
    size: SizeInt32,
    last_boot: u64,
    throttle: bool,
    dirty_regions: bool,
    /// The dirty regions of frames that never reached the main thread, throttled or dropped on
    /// a full queue. They go with the next frame, so what changed is never lost (spec 2.5).
    carried: f32,
    lost: bool,
    stopped: bool,
}

impl Handler {
    fn on_frame(&mut self, pool: &Direct3D11CaptureFramePool) -> windows::core::Result<()> {
        let frame = pool.TryGetNextFrame()?;
        let result = self.take(pool, &frame);
        let _ = frame.Close();
        if let Err(e) = &result {
            if (is_device_lost(e) || self.d3d.removed()) && !self.lost {
                self.lost = true;
                let _ = self.to_main.try_send(Ev::DeviceLost(self.generation));
            }
        }
        result
    }

    fn take(
        &mut self,
        pool: &Direct3D11CaptureFramePool,
        frame: &Direct3D11CaptureFrame,
    ) -> windows::core::Result<()> {
        if self.stopped || self.lost {
            return Ok(());
        }
        let content = frame.ContentSize()?;
        if content.Width < 2 || content.Height < 2 {
            return Ok(());
        }
        if content != self.size {
            // The monitor changed resolution or orientation: the next frames come at the new
            // size, and the main thread ends the video file for it.
            pool.Recreate(
                &self.d3d.winrt,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                content,
            )?;
            self.size = content;
            return Ok(());
        }
        let boot = clockwin::frame_boot(frame.SystemRelativeTime()?.Duration);
        let dirty = if self.dirty_regions {
            dirty_fraction(frame, content.Width as u32, content.Height as u32)
                .map(|d| carry(&mut self.carried, d))
        } else {
            None
        };
        if self.throttle
            && self.last_boot != 0
            && (boot as i64 - self.last_boot as i64) < MIN_INTERVAL_100NS
        {
            self.carried = dirty.unwrap_or(0.0);
            return Ok(());
        }
        let surface = frame.Surface()?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
        let source: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { source.GetDesc(&mut desc) };
        let width = even((content.Width as u32).min(desc.Width));
        let height = even((content.Height as u32).min(desc.Height));
        // Until the copy is handed over, this frame's changes go with the next one.
        self.carried = dirty.unwrap_or(0.0);
        let texture = self.d3d.texture(width, height)?;
        unsafe {
            self.d3d.context.CopySubresourceRegion(
                &texture,
                0,
                0,
                0,
                0,
                &source,
                0,
                Some(&D3D11_BOX {
                    left: 0,
                    top: 0,
                    front: 0,
                    right: width,
                    bottom: height,
                    back: 1,
                }),
            );
        }
        self.last_boot = boot;
        // A full queue means the main thread is behind: drop this frame rather than block a
        // thread-pool thread. Its changes go with the next one.
        let sent = self.to_main.try_send(Ev::Frame(CapturedFrame {
            picture: super::Texture(texture),
            boot,
            width,
            height,
            dirty,
            generation: self.generation,
        }));
        if sent.is_ok() {
            self.carried = 0.0;
        }
        Ok(())
    }
}

/// A frame's dirty share with what the frames before it left: added up, at most the whole screen.
/// Empties `carried`.
fn carry(carried: &mut f32, dirty: f32) -> f32 {
    (std::mem::take(carried) + dirty).min(1.0)
}

/// The fraction of the frame the dirty rectangles cover, capped at 1 (they may overlap).
fn dirty_fraction(frame: &Direct3D11CaptureFrame, width: u32, height: u32) -> Option<f32> {
    let regions = frame.DirtyRegions().ok()?;
    let mut area = 0f64;
    for r in &regions {
        area += f64::from(r.Width.max(0)) * f64::from(r.Height.max(0));
    }
    let total = f64::from(width) * f64::from(height);
    Some((area / total).min(1.0) as f32)
}
