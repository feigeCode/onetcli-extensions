use ironrdp_client::rdp::RdpOutputEvent;

use crate::pixels::rdp_u32_pixels_to_bgra;
use crate::protocol::{HelperEvent, HelperFrameRect, HelperReconnectReason};

#[derive(Default)]
pub(super) struct RdpOutputMapper {
    connected: bool,
    base_size: Option<(u16, u16)>,
    // IronRDP may publish pointer state before its first image. The main
    // process treats `Connected` as the session barrier, so retain only the
    // latest pre-connect appearance and position and flush them after it.
    pending_cursor: PendingCursor,
}

#[derive(Default)]
struct PendingCursor {
    appearance: Option<HelperEvent>,
    position: Option<(u16, u16)>,
}

impl PendingCursor {
    fn record(&mut self, event: HelperEvent) {
        match event {
            HelperEvent::CursorPosition { x, y } => self.position = Some((x, y)),
            event @ (HelperEvent::CursorDefault
            | HelperEvent::CursorHidden
            | HelperEvent::CursorRgbaBytes { .. }) => self.appearance = Some(event),
            _ => debug_assert!(false, "only cursor events may be buffered"),
        }
    }

    fn append_to(&mut self, events: &mut Vec<HelperEvent>) {
        if let Some(appearance) = self.appearance.take() {
            events.push(appearance);
        }
        if let Some((x, y)) = self.position.take() {
            events.push(HelperEvent::CursorPosition { x, y });
        }
    }

    fn clear(&mut self) {
        self.appearance = None;
        self.position = None;
    }
}

impl RdpOutputMapper {
    pub(super) fn map(&mut self, event: RdpOutputEvent) -> Vec<HelperEvent> {
        match event {
            RdpOutputEvent::Connected
            | RdpOutputEvent::LoginComplete
            | RdpOutputEvent::PostLogonDisplayRedraw
            | RdpOutputEvent::MalformedBitmapDisplayRedraw => Vec::new(),
            RdpOutputEvent::DisplayResizeFallback(reason) => {
                tracing::warn!(?reason, "RDP dynamic display resize fell back to reconnect");
                self.reset_session();
                vec![HelperEvent::Reconnecting {
                    reason: HelperReconnectReason::DisplayUpdate,
                    delay_secs: None,
                }]
            }
            RdpOutputEvent::Image {
                buffer,
                width,
                height,
            } => {
                let width = width.get();
                let height = height.get();
                let mut events = Vec::with_capacity(if self.connected { 1 } else { 4 });
                if !self.connected {
                    events.push(HelperEvent::Connected { width, height });
                    self.connected = true;
                    self.pending_cursor.append_to(&mut events);
                }
                self.base_size = Some((width, height));
                events.push(HelperEvent::frame(
                    width,
                    height,
                    rdp_u32_pixels_to_bgra(&buffer),
                ));
                events
            }
            RdpOutputEvent::ImageRegion {
                bgra,
                width,
                height,
                region,
            } => {
                if !self.connected {
                    tracing::warn!("Ignored RDP dirty region before the complete base frame");
                    return Vec::new();
                }
                let width = width.get();
                let height = height.get();
                if self.base_size != Some((width, height)) {
                    tracing::warn!(
                        width,
                        height,
                        base_size = ?self.base_size,
                        "Ignored RDP dirty region for a different base frame size"
                    );
                    return Vec::new();
                }
                let expected_len = usize::from(region.width)
                    .checked_mul(usize::from(region.height))
                    .and_then(|pixels| pixels.checked_mul(4));
                let right = u32::from(region.x) + u32::from(region.width);
                let bottom = u32::from(region.y) + u32::from(region.height);
                if region.width == 0
                    || region.height == 0
                    || right > u32::from(width)
                    || bottom > u32::from(height)
                    || expected_len != Some(bgra.len())
                {
                    tracing::warn!(
                        width,
                        height,
                        region_x = region.x,
                        region_y = region.y,
                        region_width = region.width,
                        region_height = region.height,
                        actual_bytes = bgra.len(),
                        expected_bytes = ?expected_len,
                        "Ignored malformed RDP dirty region"
                    );
                    return Vec::new();
                }
                let byte_len = bgra.len();
                vec![HelperEvent::FrameBgraRects {
                    width,
                    height,
                    rects: vec![HelperFrameRect {
                        x: region.x,
                        y: region.y,
                        width: region.width,
                        height: region.height,
                        byte_len,
                    }],
                    bgra,
                }]
            }
            RdpOutputEvent::ConnectionFailure(error) => {
                self.reset_session();
                vec![HelperEvent::ConnectionFailure {
                    message: error.report().with_locations().to_string(),
                }]
            }
            RdpOutputEvent::Terminated(result) => {
                self.reset_session();
                vec![HelperEvent::Terminated {
                    message: match result {
                        Ok(reason) => reason.to_string(),
                        Err(error) => error.report().to_string(),
                    },
                }]
            }
            RdpOutputEvent::PointerDefault => self.map_cursor(HelperEvent::CursorDefault),
            RdpOutputEvent::PointerHidden => self.map_cursor(HelperEvent::CursorHidden),
            RdpOutputEvent::PointerPosition { x, y } => {
                self.map_cursor(HelperEvent::CursorPosition { x, y })
            }
            RdpOutputEvent::PointerBitmap(pointer) => {
                if pointer.width == 0 || pointer.height == 0 {
                    self.map_cursor(HelperEvent::CursorHidden)
                } else {
                    self.map_cursor(HelperEvent::CursorRgbaBytes {
                        width: pointer.width,
                        height: pointer.height,
                        hotspot_x: pointer.hotspot_x,
                        hotspot_y: pointer.hotspot_y,
                        rgba: pointer.bitmap_data.clone(),
                    })
                }
            }
        }
    }

    fn reset_session(&mut self) {
        self.connected = false;
        self.base_size = None;
        self.pending_cursor.clear();
    }

    fn map_cursor(&mut self, event: HelperEvent) -> Vec<HelperEvent> {
        if self.connected {
            vec![event]
        } else {
            self.pending_cursor.record(event);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::num::NonZeroU16;
    use std::sync::Arc;

    use ironrdp::connector::ConnectorErrorExt as _;
    use ironrdp::graphics::pointer::DecodedPointer;
    use ironrdp_client::rdp::RdpImageRegion;

    use super::*;

    #[test]
    fn reports_connected_only_when_first_base_frame_arrives() {
        let mut mapper = RdpOutputMapper::default();
        assert_eq!(
            mapper.map(image(&[0x00112233], 1, 1)),
            vec![
                HelperEvent::Connected {
                    width: 1,
                    height: 1,
                },
                HelperEvent::frame(1, 1, vec![0x33, 0x22, 0x11, 0xff]),
            ]
        );

        assert_eq!(
            mapper.map(image(&[0x00abcdef], 1, 1)),
            vec![HelperEvent::frame(1, 1, vec![0xef, 0xcd, 0xab, 0xff])]
        );
    }

    #[test]
    fn connection_failure_includes_the_underlying_error() {
        let mut mapper = RdpOutputMapper::default();
        let error = ironrdp::connector::ConnectorError::custom(
            "TLS upgrade",
            io::Error::other("the server only offered TLS 1.0"),
        );

        let events = mapper.map(RdpOutputEvent::ConnectionFailure(error));

        let [HelperEvent::ConnectionFailure { message }] = events.as_slice() else {
            panic!("expected one connection failure event");
        };
        assert!(message.contains("[TLS upgrade @"), "{message}");
        assert!(message.contains("custom error"), "{message}");
        assert!(
            message.contains("caused by: the server only offered TLS 1.0"),
            "{message}"
        );
    }

    #[test]
    fn dirty_region_is_forwarded_without_scanning_or_converting_the_frame() {
        let mut mapper = RdpOutputMapper::default();
        mapper.map(image(&[0; 4], 2, 2));
        let bgra = vec![0x33, 0x22, 0x11, 0xff];

        assert_eq!(
            mapper.map(region(bgra.clone(), 2, 2, 1, 0, 1, 1)),
            vec![HelperEvent::FrameBgraRects {
                width: 2,
                height: 2,
                rects: vec![HelperFrameRect {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 1,
                    byte_len: bgra.len(),
                }],
                bgra,
            }]
        );
    }

    #[test]
    fn dirty_region_cannot_cross_the_connected_base_frame_barrier() {
        let mut mapper = RdpOutputMapper::default();

        assert!(
            mapper
                .map(region(vec![0, 0, 0, 0xff], 1, 1, 0, 0, 1, 1))
                .is_empty()
        );
        assert_eq!(
            mapper.map(image(&[0], 1, 1)),
            vec![
                HelperEvent::Connected {
                    width: 1,
                    height: 1,
                },
                HelperEvent::frame(1, 1, vec![0, 0, 0, 0xff]),
            ]
        );
    }

    #[test]
    fn dirty_region_must_match_the_current_base_frame() {
        let mut mapper = RdpOutputMapper::default();
        mapper.map(image(&[0; 4], 2, 2));

        assert!(
            mapper
                .map(region(vec![0, 0, 0, 0xff], 3, 2, 0, 0, 1, 1))
                .is_empty()
        );
        assert!(
            mapper
                .map(region(vec![0, 0, 0], 2, 2, 0, 0, 1, 1))
                .is_empty()
        );
        assert!(
            mapper
                .map(region(vec![0, 0, 0, 0xff], 2, 2, 2, 0, 1, 1))
                .is_empty()
        );
    }

    #[test]
    fn resize_fallback_resets_first_frame_barrier_for_reconnected_session() {
        let mut mapper = RdpOutputMapper::default();
        mapper.map(image(&[0x00112233], 1, 1));

        assert_eq!(
            mapper.map(RdpOutputEvent::DisplayResizeFallback(
                ironrdp_client::rdp::DisplayResizeFallbackReason::DisplayControlUnavailable,
            )),
            vec![HelperEvent::Reconnecting {
                reason: HelperReconnectReason::DisplayUpdate,
                delay_secs: None,
            }]
        );
        assert!(
            mapper
                .map(region(vec![0, 0, 0, 0xff], 1, 1, 0, 0, 1, 1))
                .is_empty()
        );
        assert_eq!(
            mapper.map(image(&[0x00112233], 1, 1)),
            vec![
                HelperEvent::Connected {
                    width: 1,
                    height: 1,
                },
                HelperEvent::frame(1, 1, vec![0x33, 0x22, 0x11, 0xff]),
            ]
        );
    }

    #[test]
    fn emits_decoded_pointer_bitmap_without_json_encoding_pixels() {
        let mut mapper = RdpOutputMapper::default();
        let rgba = vec![0x11, 0x22, 0x33, 0x44, 0xaa, 0xbb, 0xcc, 0xdd];
        mapper.map(image(&[0, 0], 2, 1));

        let events = mapper.map(RdpOutputEvent::PointerBitmap(Arc::new(DecodedPointer {
            width: 2,
            height: 1,
            hotspot_x: 1,
            hotspot_y: 0,
            bitmap_data: rgba.clone(),
        })));

        assert_eq!(
            events,
            vec![HelperEvent::CursorRgbaBytes {
                width: 2,
                height: 1,
                hotspot_x: 1,
                hotspot_y: 0,
                rgba,
            }]
        );
    }

    #[test]
    fn maps_zero_sized_pointer_bitmap_to_hidden_cursor() {
        let mut mapper = RdpOutputMapper::default();
        mapper.map(image(&[0], 1, 1));

        let events = mapper.map(RdpOutputEvent::PointerBitmap(Arc::new(
            DecodedPointer::new_invisible(),
        )));

        assert_eq!(events, vec![HelperEvent::CursorHidden]);
    }

    #[test]
    fn flushes_latest_preconnect_cursor_state_after_connected_and_before_frame() {
        let mut mapper = RdpOutputMapper::default();
        let rgba = vec![0x11, 0x22, 0x33, 0x44, 0xaa, 0xbb, 0xcc, 0xdd];
        assert!(
            mapper
                .map(RdpOutputEvent::PointerBitmap(Arc::new(DecodedPointer {
                    width: 2,
                    height: 1,
                    hotspot_x: 1,
                    hotspot_y: 0,
                    bitmap_data: rgba.clone(),
                })))
                .is_empty()
        );
        assert!(
            mapper
                .map(RdpOutputEvent::PointerPosition { x: 3, y: 4 })
                .is_empty()
        );

        assert_eq!(
            mapper.map(image(&[0, 0], 2, 1)),
            vec![
                HelperEvent::Connected {
                    width: 2,
                    height: 1,
                },
                HelperEvent::CursorRgbaBytes {
                    width: 2,
                    height: 1,
                    hotspot_x: 1,
                    hotspot_y: 0,
                    rgba,
                },
                HelperEvent::CursorPosition { x: 3, y: 4 },
                HelperEvent::frame(2, 1, vec![0, 0, 0, 0xff, 0, 0, 0, 0xff]),
            ]
        );
    }

    fn image(buffer: &[u32], width: u16, height: u16) -> RdpOutputEvent {
        RdpOutputEvent::Image {
            buffer: buffer.to_vec(),
            width: NonZeroU16::new(width).unwrap(),
            height: NonZeroU16::new(height).unwrap(),
        }
    }

    fn region(
        bgra: Vec<u8>,
        width: u16,
        height: u16,
        x: u16,
        y: u16,
        region_width: u16,
        region_height: u16,
    ) -> RdpOutputEvent {
        RdpOutputEvent::ImageRegion {
            bgra,
            width: NonZeroU16::new(width).unwrap(),
            height: NonZeroU16::new(height).unwrap(),
            region: RdpImageRegion {
                x,
                y,
                width: region_width,
                height: region_height,
            },
        }
    }
}
