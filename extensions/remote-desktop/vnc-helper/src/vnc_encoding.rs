use std::time::{Duration, Instant};

use vnc_client::{Rect, VncClient, VncEvent, X11Event};

use crate::framebuffer::RgbaFramebuffer;
use crate::output_mailbox::OutputSender;
use crate::runtime::{
    RemoteDesktopCapabilities, RemoteDesktopFrameRect, RemoteDesktopOutput, ResizeSupport,
};
use crate::vnc_input::VncPointerState;
use crate::vnc_keyboard::VncKeyboardState;

const VNC_REFRESH_INTERVAL: Duration = Duration::from_millis(33);
const VNC_REFRESH_RESPONSE_TIMEOUT: Duration = Duration::from_millis(500);
const VNC_LIVENESS_TIMEOUT: Duration = Duration::from_secs(15);
const VNC_LIVENESS_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefreshAction {
    Wait,
    Incremental,
    LivenessProbe,
    Reconnect,
}

pub(crate) struct ConnectedVncSession {
    pub(crate) client: VncClient,
    pub(crate) keyboard: VncKeyboardState,
    pub(crate) pointer: VncPointerState,
    pub(crate) was_connected: bool,
    framebuffer: VncFramebufferState,
    last_refresh: Instant,
    last_server_activity: Instant,
    refresh_in_flight_since: Option<Instant>,
    liveness_probe_since: Option<Instant>,
}

impl ConnectedVncSession {
    pub(crate) fn new(client: VncClient) -> Self {
        let now = Instant::now();
        Self {
            client,
            keyboard: VncKeyboardState::default(),
            pointer: VncPointerState::default(),
            was_connected: false,
            framebuffer: VncFramebufferState::default(),
            last_refresh: now,
            last_server_activity: now,
            refresh_in_flight_since: None,
            liveness_probe_since: None,
        }
    }

    pub(crate) async fn request_initial_refresh(&self) -> Result<(), String> {
        self.client
            .input(X11Event::FullRefresh)
            .await
            .map_err(|error| error.to_string())
    }

    pub(crate) async fn poll_events(&mut self, output_tx: &OutputSender) -> Result<(), String> {
        loop {
            match self.client.poll_event().await {
                Ok(Some(event)) => {
                    self.last_server_activity = Instant::now();
                    self.refresh_in_flight_since = None;
                    self.liveness_probe_since = None;
                    self.handle_event(event, output_tx)?;
                }
                Ok(None) => {
                    self.framebuffer.flush_frame(output_tx);
                    return Ok(());
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }

    pub(crate) async fn refresh_if_needed(&mut self) -> Result<(), String> {
        let now = Instant::now();
        match refresh_action(
            now,
            self.last_refresh,
            self.last_server_activity,
            self.refresh_in_flight_since,
            self.liveness_probe_since,
        ) {
            RefreshAction::Wait => Ok(()),
            RefreshAction::Reconnect => Err("VNC liveness probe timed out".to_string()),
            RefreshAction::Incremental => {
                self.client
                    .input(X11Event::Refresh)
                    .await
                    .map_err(|error| error.to_string())?;
                self.last_refresh = now;
                self.refresh_in_flight_since = Some(now);
                Ok(())
            }
            RefreshAction::LivenessProbe => {
                self.client
                    .input(X11Event::FullRefresh)
                    .await
                    .map_err(|error| error.to_string())?;
                self.liveness_probe_since = Some(now);
                self.refresh_in_flight_since = Some(now);
                self.last_refresh = now;
                Ok(())
            }
        }
    }

    fn handle_event(&mut self, event: VncEvent, output_tx: &OutputSender) -> Result<(), String> {
        match event {
            VncEvent::SetResolution(screen) => self.set_resolution(screen, output_tx),
            VncEvent::RawImage(rect, data) => self.patch_rect(rect, &data)?,
            VncEvent::Copy(dst, src) => self.copy_rect(dst, src)?,
            VncEvent::Text(text) => send_clipboard(output_tx, text),
            VncEvent::Error(message) => return Err(message),
            VncEvent::JpegImage(_, _) => {
                send_status(output_tx, "VNC JPEG rectangles are not enabled")
            }
            VncEvent::Bell | VncEvent::SetPixelFormat(_) | VncEvent::SetCursor(_, _) => {}
            _ => {}
        }
        Ok(())
    }

    fn set_resolution(&mut self, screen: vnc_client::Screen, output_tx: &OutputSender) {
        self.framebuffer.set_resolution(screen, output_tx);
        self.was_connected = true;
    }

    fn patch_rect(&mut self, rect: Rect, data: &[u8]) -> Result<(), String> {
        self.framebuffer.patch_rect(rect, data)
    }

    fn copy_rect(&mut self, dst: Rect, src: Rect) -> Result<(), String> {
        self.framebuffer.copy_rect(dst, src)
    }
}

fn refresh_action(
    now: Instant,
    last_refresh: Instant,
    last_server_activity: Instant,
    refresh_in_flight_since: Option<Instant>,
    liveness_probe_since: Option<Instant>,
) -> RefreshAction {
    if let Some(probe_since) = liveness_probe_since {
        return if now.duration_since(probe_since) >= VNC_LIVENESS_PROBE_TIMEOUT {
            RefreshAction::Reconnect
        } else {
            RefreshAction::Wait
        };
    }

    if refresh_in_flight_since
        .is_some_and(|sent_at| now.duration_since(sent_at) < VNC_REFRESH_RESPONSE_TIMEOUT)
    {
        return RefreshAction::Wait;
    }

    if now.duration_since(last_server_activity) >= VNC_LIVENESS_TIMEOUT {
        return RefreshAction::LivenessProbe;
    }

    if now.duration_since(last_refresh) < VNC_REFRESH_INTERVAL {
        return RefreshAction::Wait;
    }

    RefreshAction::Incremental
}

#[derive(Default)]
struct VncFramebufferState {
    framebuffer: Option<RgbaFramebuffer>,
    keyframe: bool,
    dirty_rects: Vec<Rect>,
}

impl VncFramebufferState {
    fn set_resolution(&mut self, screen: vnc_client::Screen, output_tx: &OutputSender) {
        self.framebuffer = Some(RgbaFramebuffer::new(screen.width, screen.height));
        self.keyframe = true;
        self.dirty_rects.clear();
        let _ = output_tx.send(RemoteDesktopOutput::Connected {
            width: screen.width,
            height: screen.height,
            capabilities: vnc_capabilities(),
        });
    }

    fn patch_rect(&mut self, rect: Rect, data: &[u8]) -> Result<(), String> {
        let Some(framebuffer) = &mut self.framebuffer else {
            return Ok(());
        };
        framebuffer
            .patch_rgba_rect(rect.x, rect.y, rect.width, rect.height, data)
            .map_err(|error| error.to_string())?;
        self.dirty_rects.push(rect);
        Ok(())
    }

    fn copy_rect(&mut self, dst: Rect, src: Rect) -> Result<(), String> {
        let Some(framebuffer) = &mut self.framebuffer else {
            return Ok(());
        };
        framebuffer
            .copy_rect(src.x, src.y, dst.x, dst.y, dst.width, dst.height)
            .map_err(|error| error.to_string())?;
        self.dirty_rects.push(dst);
        Ok(())
    }

    fn flush_frame(&mut self, output_tx: &OutputSender) {
        let Some(framebuffer) = &self.framebuffer else {
            return;
        };
        if self.dirty_rects.is_empty() {
            return;
        }
        if self.keyframe {
            let _ = output_tx.send(RemoteDesktopOutput::Frame {
                width: framebuffer.width(),
                height: framebuffer.height(),
                rgba: framebuffer.clone_bgra(),
            });
            self.keyframe = false;
            self.dirty_rects.clear();
            return;
        }

        let rects: Vec<RemoteDesktopFrameRect> = self
            .dirty_rects
            .drain(..)
            .map(|rect| RemoteDesktopFrameRect {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: rect.height,
                byte_len: usize::from(rect.width) * usize::from(rect.height) * 4,
            })
            .collect();
        let mut bgra = Vec::new();
        for rect in &rects {
            bgra.extend(framebuffer.clone_bgra_rect(rect.x, rect.y, rect.width, rect.height));
        }
        let _ = output_tx.send(RemoteDesktopOutput::FrameBgraRects {
            width: framebuffer.width(),
            height: framebuffer.height(),
            rects,
            bgra,
        });
    }
}

fn send_clipboard(output_tx: &OutputSender, text: String) {
    let _ = output_tx.send(RemoteDesktopOutput::ClipboardText { text });
}

fn vnc_capabilities() -> RemoteDesktopCapabilities {
    RemoteDesktopCapabilities {
        resize: ResizeSupport::LocalScaleOnly,
        clipboard_text: true,
        ..RemoteDesktopCapabilities::vnc_mvp()
    }
}

fn send_status(output_tx: &OutputSender, message: &str) {
    let _ = output_tx.send(RemoteDesktopOutput::Status(message.to_string()));
}

#[cfg(test)]
#[path = "vnc_encoding_tests.rs"]
mod tests;
