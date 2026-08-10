use ironrdp_client::rdp::RdpOutputEvent;

use crate::pixels::rdp_u32_pixels_to_bgra;
use crate::protocol::{HelperEvent, HelperFrameRect, HelperReconnectReason};

const DIRTY_TILE_SIZE: usize = 64;
const FULL_FRAME_THRESHOLD_PERCENT: usize = 60;
const MAX_ACCUMULATED_DIRTY_AREA_PERCENT: usize = 50;
const MAX_ACCUMULATED_DIRTY_RECTS: usize = 256;
const BGRA_BYTES_PER_PIXEL: usize = 4;

#[derive(Default)]
pub(super) struct RdpOutputMapper {
    connected: bool,
    previous: Option<PreviousFrame>,
    delta_budget: DeltaBudget,
    // IronRDP may publish pointer state before its first image. The main
    // process treats `Connected` as the session barrier, so retain only the
    // latest pre-connect appearance and position and flush them after it.
    pending_cursor: PendingCursor,
}

struct PreviousFrame {
    width: u16,
    height: u16,
    pixels: Vec<u32>,
}

#[derive(Default)]
struct DeltaBudget {
    rect_count: usize,
    dirty_area: usize,
}

impl DeltaBudget {
    fn record(&mut self, rect_count: usize, dirty_area: usize) {
        self.rect_count = self.rect_count.saturating_add(rect_count);
        self.dirty_area = self.dirty_area.saturating_add(dirty_area);
    }

    fn requires_full_frame(&self, total_area: usize) -> bool {
        self.rect_count >= MAX_ACCUMULATED_DIRTY_RECTS
            || percentage_reached(
                self.dirty_area,
                total_area,
                MAX_ACCUMULATED_DIRTY_AREA_PERCENT,
            )
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

fn percentage_reached(value: usize, total: usize, percent: usize) -> bool {
    (value as u128) * 100 >= (total as u128) * (percent as u128)
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
                let frame = self.map_frame(width, height, &buffer);
                self.previous = Some(PreviousFrame {
                    width,
                    height,
                    pixels: buffer,
                });
                if let Some(frame) = frame {
                    events.push(frame);
                }
                events
            }
            RdpOutputEvent::ConnectionFailure(error) => {
                self.reset_session();
                vec![HelperEvent::ConnectionFailure {
                    message: format!("{error:#}"),
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
        self.previous = None;
        self.delta_budget.reset();
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

    fn map_frame(&mut self, width: u16, height: u16, pixels: &[u32]) -> Option<HelperEvent> {
        let Some(previous) = self.previous.as_ref() else {
            self.delta_budget.reset();
            return Some(HelperEvent::frame(
                width,
                height,
                rdp_u32_pixels_to_bgra(pixels),
            ));
        };
        if previous.width != width || previous.height != height {
            self.delta_budget.reset();
            return Some(HelperEvent::frame(
                width,
                height,
                rdp_u32_pixels_to_bgra(pixels),
            ));
        }

        let rects = dirty_rects(&previous.pixels, pixels, width as usize, height as usize);
        if rects.is_empty() {
            return None;
        }
        let changed_area: usize = rects
            .iter()
            .map(|rect| usize::from(rect.width) * usize::from(rect.height))
            .sum();
        let total_area = usize::from(width) * usize::from(height);
        let payload_bytes = changed_area.saturating_mul(BGRA_BYTES_PER_PIXEL);
        self.delta_budget.record(rects.len(), changed_area);
        if percentage_reached(changed_area, total_area, FULL_FRAME_THRESHOLD_PERCENT)
            || self.delta_budget.requires_full_frame(total_area)
        {
            self.delta_budget.reset();
            return Some(HelperEvent::frame(
                width,
                height,
                rdp_u32_pixels_to_bgra(pixels),
            ));
        }

        let mut bgra = Vec::with_capacity(payload_bytes);
        for rect in &rects {
            append_rect_bgra(&mut bgra, pixels, width as usize, rect);
        }
        Some(HelperEvent::FrameBgraRects {
            width,
            height,
            rects,
            bgra,
        })
    }
}

fn dirty_rects(
    previous: &[u32],
    current: &[u32],
    width: usize,
    height: usize,
) -> Vec<HelperFrameRect> {
    let tiles_x = width.div_ceil(DIRTY_TILE_SIZE);
    let tiles_y = height.div_ceil(DIRTY_TILE_SIZE);
    let mut rects = Vec::new();
    for tile_y in 0..tiles_y {
        let mut tile_x = 0;
        while tile_x < tiles_x {
            while tile_x < tiles_x
                && !tile_changed(previous, current, width, height, tile_x, tile_y)
            {
                tile_x += 1;
            }
            if tile_x == tiles_x {
                break;
            }
            let start_x = tile_x;
            tile_x += 1;
            while tile_x < tiles_x && tile_changed(previous, current, width, height, tile_x, tile_y)
            {
                tile_x += 1;
            }
            let x = start_x * DIRTY_TILE_SIZE;
            let y = tile_y * DIRTY_TILE_SIZE;
            let rect_width = (tile_x * DIRTY_TILE_SIZE).min(width) - x;
            let rect_height = ((tile_y + 1) * DIRTY_TILE_SIZE).min(height) - y;
            rects.push(HelperFrameRect {
                x: x as u16,
                y: y as u16,
                width: rect_width as u16,
                height: rect_height as u16,
                byte_len: rect_width * rect_height * 4,
            });
        }
    }
    rects
}

fn tile_changed(
    previous: &[u32],
    current: &[u32],
    width: usize,
    height: usize,
    tile_x: usize,
    tile_y: usize,
) -> bool {
    let start_x = tile_x * DIRTY_TILE_SIZE;
    let start_y = tile_y * DIRTY_TILE_SIZE;
    let end_x = (start_x + DIRTY_TILE_SIZE).min(width);
    let end_y = (start_y + DIRTY_TILE_SIZE).min(height);
    (start_y..end_y).any(|y| {
        let row = y * width;
        (start_x..end_x).any(|x| previous[row + x] != current[row + x])
    })
}

fn append_rect_bgra(
    output: &mut Vec<u8>,
    pixels: &[u32],
    framebuffer_width: usize,
    rect: &HelperFrameRect,
) {
    let x = usize::from(rect.x);
    let y = usize::from(rect.y);
    let width = usize::from(rect.width);
    let height = usize::from(rect.height);
    for row in 0..height {
        let start = (y + row) * framebuffer_width + x;
        for pixel in &pixels[start..start + width] {
            let [_, r, g, b] = pixel.to_be_bytes();
            output.extend_from_slice(&[b, g, r, 0xff]);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironrdp::graphics::pointer::DecodedPointer;

    use super::*;

    #[test]
    fn reports_connected_only_when_first_frame_arrives() {
        let mut mapper = RdpOutputMapper::default();
        let first = mapper.map(RdpOutputEvent::Image {
            buffer: vec![0x00112233],
            width: std::num::NonZeroU16::new(1).unwrap(),
            height: std::num::NonZeroU16::new(1).unwrap(),
        });

        assert_eq!(
            first,
            vec![
                HelperEvent::Connected {
                    width: 1,
                    height: 1
                },
                HelperEvent::frame(1, 1, vec![0x33, 0x22, 0x11, 0xff])
            ]
        );

        let second = mapper.map(RdpOutputEvent::Image {
            buffer: vec![0x00abcdef],
            width: std::num::NonZeroU16::new(1).unwrap(),
            height: std::num::NonZeroU16::new(1).unwrap(),
        });
        assert_eq!(
            second,
            vec![HelperEvent::frame(1, 1, vec![0xef, 0xcd, 0xab, 0xff])]
        );
    }

    #[test]
    fn ironrdp_connected_does_not_cross_host_connected_barrier() {
        let mut mapper = RdpOutputMapper::default();

        assert!(mapper.map(RdpOutputEvent::Connected).is_empty());
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
        assert!(mapper.map(RdpOutputEvent::Connected).is_empty());
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
    fn emits_dirty_rectangles_for_small_screen_updates() {
        let mut mapper = RdpOutputMapper::default();
        let mut first_pixels = vec![0; 128 * 128];
        let first = mapper.map(image(&first_pixels, 128, 128));
        assert!(matches!(
            first.last(),
            Some(HelperEvent::FrameBgraBytes { .. })
        ));

        first_pixels[65 * 128 + 65] = 0x00112233;
        let second = mapper.map(image(&first_pixels, 128, 128));

        assert!(matches!(
            second.as_slice(),
            [HelperEvent::FrameBgraRects {
                width: 128,
                height: 128,
                ..
            }]
        ));
    }

    #[test]
    fn accumulated_dirty_area_promotes_to_a_full_frame_and_resets() {
        let mut mapper = RdpOutputMapper::default();
        let mut pixels = vec![0; 128 * 128];
        mapper.map(image(&pixels, 128, 128));

        pixels[1] = 0x00112233;
        assert!(matches!(
            mapper.map(image(&pixels, 128, 128)).as_slice(),
            [HelperEvent::FrameBgraRects { .. }]
        ));

        pixels[65] = 0x00445566;
        assert!(matches!(
            mapper.map(image(&pixels, 128, 128)).as_slice(),
            [HelperEvent::FrameBgraBytes { .. }]
        ));

        pixels[128 * 65] = 0x00778899;
        assert!(matches!(
            mapper.map(image(&pixels, 128, 128)).as_slice(),
            [HelperEvent::FrameBgraRects { .. }]
        ));
    }

    #[test]
    fn accumulated_delta_budget_enforces_the_rectangle_limit() {
        let mut budget = DeltaBudget::default();
        budget.rect_count = MAX_ACCUMULATED_DIRTY_RECTS - 1;

        budget.record(1, 1);

        assert!(budget.requires_full_frame(usize::MAX / 100));
    }

    #[test]
    fn session_reset_discards_previous_frame_and_delta_budget() {
        let mut mapper = RdpOutputMapper::default();
        mapper.connected = true;
        mapper.previous = Some(PreviousFrame {
            width: 1,
            height: 1,
            pixels: vec![0x00112233],
        });
        mapper.delta_budget.record(12, 34);
        mapper
            .pending_cursor
            .record(HelperEvent::CursorPosition { x: 1, y: 2 });

        mapper.reset_session();

        assert!(!mapper.connected);
        assert!(mapper.previous.is_none());
        assert_eq!(mapper.delta_budget.rect_count, 0);
        assert_eq!(mapper.delta_budget.dirty_area, 0);
        assert!(mapper.pending_cursor.position.is_none());
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
        let stale_rgba = vec![0x10, 0x20, 0x30, 0x40];
        let rgba = vec![0x11, 0x22, 0x33, 0x44, 0xaa, 0xbb, 0xcc, 0xdd];

        assert!(
            mapper
                .map(RdpOutputEvent::PointerBitmap(Arc::new(DecodedPointer {
                    width: 1,
                    height: 1,
                    hotspot_x: 0,
                    hotspot_y: 0,
                    bitmap_data: stale_rgba,
                })))
                .is_empty()
        );
        assert!(
            mapper
                .map(RdpOutputEvent::PointerPosition { x: 1, y: 2 })
                .is_empty()
        );
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
            width: std::num::NonZeroU16::new(width).unwrap(),
            height: std::num::NonZeroU16::new(height).unwrap(),
        }
    }

    #[test]
    #[ignore = "manual performance benchmark"]
    fn benchmarks_sparse_frame_transport() {
        const WIDTH: u16 = 1280;
        const HEIGHT: u16 = 720;
        const FRAMES: usize = 60;
        let pixels = usize::from(WIDTH) * usize::from(HEIGHT);
        let baseline_frame = vec![0u32; pixels];

        let baseline_started = std::time::Instant::now();
        for _ in 0..FRAMES {
            std::hint::black_box(rdp_u32_pixels_to_bgra(&baseline_frame));
        }
        let baseline_elapsed = baseline_started.elapsed();

        let optimized_started = std::time::Instant::now();
        let mut mapper = RdpOutputMapper::default();
        let mut optimized_bytes = 0usize;
        for frame in 0..FRAMES {
            let mut pixels = baseline_frame.clone();
            let x = (frame * 17) % usize::from(WIDTH);
            let y = (frame * 11) % usize::from(HEIGHT);
            pixels[y * usize::from(WIDTH) + x] = 0x00112233;
            for event in mapper.map(image(&pixels, WIDTH, HEIGHT)) {
                optimized_bytes += match event {
                    HelperEvent::FrameBgraBytes { bgra, .. }
                    | HelperEvent::FrameBgraRects { bgra, .. } => bgra.len(),
                    _ => 0,
                };
            }
        }
        let optimized_elapsed = optimized_started.elapsed();
        let baseline_bytes = pixels * 4 * FRAMES;

        println!(
            "baseline_ms={} optimized_ms={} baseline_bytes={} optimized_bytes={}",
            baseline_elapsed.as_secs_f64() * 1000.0,
            optimized_elapsed.as_secs_f64() * 1000.0,
            baseline_bytes,
            optimized_bytes
        );
    }
}
