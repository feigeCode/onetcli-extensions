use std::future;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use ironrdp_client::rdp::{RdpClient, RdpOutputEvent};
use tokio::sync::mpsc;

use crate::clipboard::{TextClipboardController, text_clipboard};
use crate::output_mailbox::{OutputReceiver, OutputSender, output_mailbox};
use crate::protocol::{ConnectRequest, HelperEvent};
use crate::threading::join_worker;

mod config;
mod input;
mod input_sender;
mod output;

pub(crate) use input::{RdpInputAction, RdpInputContext, apply_input_request, shutdown_inputs};
pub(crate) use input_sender::{HelperInputSender, InputQueueStatus};
use output::RdpOutputMapper;

// IronRDP can emit several complete framebuffer snapshots for one logical
// desktop update. Keep a short quiet window so those snapshots collapse to the
// newest one, but cap a continuous animation at roughly one 60 Hz frame rather
// than the previous 33 ms / ~30 FPS cadence.
const FRAME_SETTLE_INTERVAL: Duration = Duration::from_millis(4);
const MAX_FRAME_PRESENTATION_LATENCY: Duration = Duration::from_millis(16);
const FRAME_PACING_LOG_INTERVAL: Duration = Duration::from_secs(1);

pub struct RdpRuntime {
    pub input_tx: HelperInputSender,
    output_rx: Option<OutputReceiver>,
    pub clipboard: TextClipboardController,
    client_thread: std::thread::JoinHandle<anyhow::Result<()>>,
}

pub fn start(connect: ConnectRequest) -> anyhow::Result<RdpRuntime> {
    let config = config::build_config(connect)?;
    let (output_tx, output_rx) = mpsc::channel::<RdpOutputEvent>(64);
    let (helper_output_tx, helper_output_rx) = output_mailbox();
    let client = RdpClient::new(config, output_tx);
    let input_tx = HelperInputSender::production(client.input_sender());
    let (clipboard, cliprdr_factory) = text_clipboard(input_tx.clone(), helper_output_tx.clone());
    let client = client.with_cliprdr_backend_factory(cliprdr_factory);

    let client_thread = spawn_client_thread(client, output_rx, helper_output_tx)?;
    Ok(RdpRuntime {
        input_tx,
        output_rx: Some(helper_output_rx),
        clipboard,
        client_thread,
    })
}

impl RdpRuntime {
    pub fn take_output_receiver(&mut self) -> anyhow::Result<OutputReceiver> {
        self.output_rx
            .take()
            .context("RDP output receiver was already taken")
    }

    pub fn shutdown(self, database: &mut ironrdp::input::Database) -> anyhow::Result<()> {
        let Self {
            input_tx,
            output_rx: _output_rx,
            clipboard,
            client_thread,
        } = self;
        if let Err(error) = shutdown_inputs(database, &input_tx) {
            tracing::debug!(?error, "RDP input channel was already closed");
        }
        clipboard.shutdown();
        drop(clipboard);
        drop(input_tx);
        join_worker(client_thread, "RDP client")
    }
}

fn spawn_client_thread(
    client: RdpClient,
    output_rx: mpsc::Receiver<RdpOutputEvent>,
    helper_output_tx: OutputSender,
) -> anyhow::Result<std::thread::JoinHandle<anyhow::Result<()>>> {
    let thread = std::thread::Builder::new()
        .name("navop-rdp-helper-runtime".to_string())
        .spawn(move || run_client_thread(client, output_rx, helper_output_tx))
        .context("spawn RDP client thread")?;
    Ok(thread)
}

fn run_client_thread(
    client: RdpClient,
    output_rx: mpsc::Receiver<RdpOutputEvent>,
    helper_output_tx: OutputSender,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build();
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            report_runtime_failure(&helper_output_tx);
            return Err(error).context("create RDP tokio runtime");
        }
    };
    runtime.block_on(run_client(client, output_rx, helper_output_tx))
}

async fn run_client(
    client: RdpClient,
    output_rx: mpsc::Receiver<RdpOutputEvent>,
    helper_output_tx: OutputSender,
) -> anyhow::Result<()> {
    let output_task = tokio::spawn(map_output_events(output_rx, helper_output_tx));
    client.run().await;
    output_task
        .await
        .context("RDP output mapper task panicked")?
}

async fn map_output_events(
    mut output_rx: mpsc::Receiver<RdpOutputEvent>,
    helper_output_tx: OutputSender,
) -> anyhow::Result<()> {
    let mut output_mapper = RdpOutputMapper::default();
    let mut pending_image = None;
    let mut presentation_schedule = FramePresentationSchedule::default();
    let mut pacing_stats = FramePacingStats::new();
    loop {
        tokio::select! {
            biased;
            event = output_rx.recv() => {
                match event {
                    Some(image @ RdpOutputEvent::Image { .. }) => {
                        pacing_stats.record_received(pending_image.replace(image).is_some());
                        presentation_schedule.record_image(tokio::time::Instant::now());
                    }
                    Some(event @ (RdpOutputEvent::PointerDefault
                    | RdpOutputEvent::PointerHidden
                    | RdpOutputEvent::PointerPosition { .. }
                    | RdpOutputEvent::PointerBitmap(_))) => {
                        publish_output_event(
                            &mut output_mapper,
                            event,
                            &helper_output_tx,
                            &mut pacing_stats,
                        )?;
                    }
                    Some(event) => {
                        flush_pending_image(
                            &mut pending_image,
                            &mut presentation_schedule,
                            &mut output_mapper,
                            &helper_output_tx,
                            &mut pacing_stats,
                        )?;
                        publish_output_event(
                            &mut output_mapper,
                            event,
                            &helper_output_tx,
                            &mut pacing_stats,
                        )?;
                    }
                    None => {
                        flush_pending_image(
                            &mut pending_image,
                            &mut presentation_schedule,
                            &mut output_mapper,
                            &helper_output_tx,
                            &mut pacing_stats,
                        )?;
                        pacing_stats.maybe_log(true);
                        break;
                    }
                }
            }
            () = async {
                match presentation_schedule.deadline() {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => future::pending().await,
                }
            } => {
                flush_pending_image(
                    &mut pending_image,
                    &mut presentation_schedule,
                    &mut output_mapper,
                    &helper_output_tx,
                    &mut pacing_stats,
                )?;
            }
        }
        if presentation_schedule.is_due(tokio::time::Instant::now()) {
            flush_pending_image(
                &mut pending_image,
                &mut presentation_schedule,
                &mut output_mapper,
                &helper_output_tx,
                &mut pacing_stats,
            )?;
        }
        pacing_stats.maybe_log(false);
    }
    Ok(())
}

#[derive(Default)]
struct FramePresentationSchedule {
    settle_deadline: Option<tokio::time::Instant>,
    max_latency_deadline: Option<tokio::time::Instant>,
}

impl FramePresentationSchedule {
    fn record_image(&mut self, now: tokio::time::Instant) {
        self.settle_deadline = Some(now + FRAME_SETTLE_INTERVAL);
        self.max_latency_deadline
            .get_or_insert(now + MAX_FRAME_PRESENTATION_LATENCY);
    }

    fn deadline(&self) -> Option<tokio::time::Instant> {
        match (self.settle_deadline, self.max_latency_deadline) {
            (Some(settle), Some(max_latency)) => Some(settle.min(max_latency)),
            (Some(settle), None) => Some(settle),
            (None, Some(max_latency)) => Some(max_latency),
            (None, None) => None,
        }
    }

    fn is_due(&self, now: tokio::time::Instant) -> bool {
        self.deadline().is_some_and(|deadline| deadline <= now)
    }

    fn clear(&mut self) {
        self.settle_deadline = None;
        self.max_latency_deadline = None;
    }
}

fn flush_pending_image(
    pending_image: &mut Option<RdpOutputEvent>,
    presentation_schedule: &mut FramePresentationSchedule,
    output_mapper: &mut RdpOutputMapper,
    helper_output_tx: &OutputSender,
    pacing_stats: &mut FramePacingStats,
) -> anyhow::Result<()> {
    presentation_schedule.clear();
    let Some(image) = pending_image.take() else {
        return Ok(());
    };
    pacing_stats.images_presented = pacing_stats.images_presented.saturating_add(1);
    publish_output_event(output_mapper, image, helper_output_tx, pacing_stats)
}

fn publish_output_event(
    output_mapper: &mut RdpOutputMapper,
    event: RdpOutputEvent,
    helper_output_tx: &OutputSender,
    pacing_stats: &mut FramePacingStats,
) -> anyhow::Result<()> {
    let map_started_at = Instant::now();
    let helper_events = output_mapper.map(event);
    pacing_stats.mapper_elapsed = pacing_stats
        .mapper_elapsed
        .saturating_add(map_started_at.elapsed());
    pacing_stats.record_helper_events(&helper_events);
    for helper_event in helper_events {
        helper_output_tx
            .send(helper_event)
            .context("RDP helper output receiver closed")?;
    }
    Ok(())
}

struct FramePacingStats {
    window_started_at: Instant,
    images_received: u64,
    images_coalesced: u64,
    images_presented: u64,
    full_frames: u64,
    delta_frames: u64,
    dirty_rects: u64,
    payload_bytes: u64,
    mapper_elapsed: Duration,
}

impl FramePacingStats {
    fn new() -> Self {
        Self {
            window_started_at: Instant::now(),
            images_received: 0,
            images_coalesced: 0,
            images_presented: 0,
            full_frames: 0,
            delta_frames: 0,
            dirty_rects: 0,
            payload_bytes: 0,
            mapper_elapsed: Duration::ZERO,
        }
    }

    fn record_received(&mut self, coalesced: bool) {
        self.images_received = self.images_received.saturating_add(1);
        self.images_coalesced = self.images_coalesced.saturating_add(u64::from(coalesced));
    }

    fn record_helper_events(&mut self, events: &[HelperEvent]) {
        for event in events {
            match event {
                HelperEvent::FrameBgraBytes { bgra, .. } => {
                    self.full_frames = self.full_frames.saturating_add(1);
                    self.payload_bytes = self.payload_bytes.saturating_add(bgra.len() as u64);
                }
                HelperEvent::FrameBgraRects { rects, bgra, .. } => {
                    self.delta_frames = self.delta_frames.saturating_add(1);
                    self.dirty_rects = self.dirty_rects.saturating_add(rects.len() as u64);
                    self.payload_bytes = self.payload_bytes.saturating_add(bgra.len() as u64);
                }
                _ => {}
            }
        }
    }

    fn maybe_log(&mut self, force: bool) {
        let elapsed = self.window_started_at.elapsed();
        if !force && elapsed < FRAME_PACING_LOG_INTERVAL {
            return;
        }
        if self.images_received > 0 {
            tracing::debug!(
                settle_interval_ms = FRAME_SETTLE_INTERVAL.as_millis() as u64,
                max_latency_ms = MAX_FRAME_PRESENTATION_LATENCY.as_millis() as u64,
                elapsed_ms = elapsed.as_millis() as u64,
                images_received = self.images_received,
                images_coalesced = self.images_coalesced,
                images_presented = self.images_presented,
                full_frames = self.full_frames,
                delta_frames = self.delta_frames,
                dirty_rects = self.dirty_rects,
                payload_bytes = self.payload_bytes,
                mapper_elapsed_us = self.mapper_elapsed.as_micros() as u64,
                "RDP frame pacing statistics"
            );
        }
        self.window_started_at = Instant::now();
        self.images_received = 0;
        self.images_coalesced = 0;
        self.images_presented = 0;
        self.full_frames = 0;
        self.delta_frames = 0;
        self.dirty_rects = 0;
        self.payload_bytes = 0;
        self.mapper_elapsed = Duration::ZERO;
    }
}

fn report_runtime_failure(helper_output_tx: &OutputSender) {
    let event = HelperEvent::ConnectionFailure {
        message: "failed to create RDP tokio runtime".to_string(),
    };
    if helper_output_tx.send(event).is_err() {
        tracing::debug!("RDP helper output receiver was already closed");
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use super::*;

    #[test]
    fn runtime_uses_coalescing_output_receiver() {
        fn assert_receiver(runtime: &RdpRuntime) {
            let _: &Option<crate::output_mailbox::OutputReceiver> = &runtime.output_rx;
        }
        let _contract: fn(&RdpRuntime) = assert_receiver;
    }

    #[tokio::test]
    async fn image_burst_presents_only_the_latest_frame() {
        let (output_tx, output_rx) = mpsc::channel(8);
        let (helper_output_tx, helper_output_rx) = output_mailbox();
        let mapper = tokio::spawn(map_output_events(output_rx, helper_output_tx));
        for red in [1, 2, 3] {
            output_tx
                .send(RdpOutputEvent::Image {
                    buffer: vec![u32::from_be_bytes([0, red, 0, 0])],
                    width: NonZeroU16::new(1).unwrap(),
                    height: NonZeroU16::new(1).unwrap(),
                })
                .await
                .unwrap();
        }
        drop(output_tx);

        mapper.await.unwrap().unwrap();
        assert_eq!(
            helper_output_rx.recv(),
            Some(HelperEvent::Connected {
                width: 1,
                height: 1
            })
        );
        assert_eq!(
            helper_output_rx.recv(),
            Some(HelperEvent::FrameBgraBytes {
                width: 1,
                height: 1,
                bgra: vec![0, 0, 3, 255],
            })
        );
        assert_eq!(helper_output_rx.recv(), None);
    }

    #[test]
    fn presentation_schedule_waits_for_a_quiet_interval() {
        let started_at = tokio::time::Instant::now();
        let mut schedule = FramePresentationSchedule::default();

        schedule.record_image(started_at);
        assert_eq!(
            schedule.deadline(),
            Some(started_at + FRAME_SETTLE_INTERVAL)
        );

        let second_image_at = started_at + Duration::from_millis(3);
        schedule.record_image(second_image_at);

        assert_eq!(
            schedule.deadline(),
            Some(second_image_at + FRAME_SETTLE_INTERVAL)
        );
        assert!(!schedule.is_due(started_at + FRAME_SETTLE_INTERVAL));
        assert!(schedule.is_due(second_image_at + FRAME_SETTLE_INTERVAL));
    }

    #[test]
    fn presentation_schedule_caps_a_continuous_burst() {
        let started_at = tokio::time::Instant::now();
        let mut schedule = FramePresentationSchedule::default();
        schedule.record_image(started_at);

        for elapsed_ms in [3, 6, 9, 12, 15] {
            schedule.record_image(started_at + Duration::from_millis(elapsed_ms));
        }

        assert_eq!(
            schedule.deadline(),
            Some(started_at + MAX_FRAME_PRESENTATION_LATENCY)
        );
        assert!(
            !schedule
                .is_due(started_at + MAX_FRAME_PRESENTATION_LATENCY - Duration::from_millis(1))
        );
        assert!(schedule.is_due(started_at + MAX_FRAME_PRESENTATION_LATENCY));
    }

    #[test]
    fn continuous_animation_is_capped_at_a_60hz_cadence() {
        let started_at = tokio::time::Instant::now();
        let mut schedule = FramePresentationSchedule::default();
        schedule.record_image(started_at);

        for elapsed_ms in (1..MAX_FRAME_PRESENTATION_LATENCY.as_millis() as u64).step_by(2) {
            schedule.record_image(started_at + Duration::from_millis(elapsed_ms));
        }

        let presentation_deadline = schedule.deadline().unwrap();
        assert_eq!(
            presentation_deadline,
            started_at + MAX_FRAME_PRESENTATION_LATENCY
        );
        assert!(
            presentation_deadline.duration_since(started_at) <= Duration::from_millis(16),
            "continuous image bursts must not be throttled below 60 Hz"
        );
    }

    #[tokio::test]
    async fn settled_image_is_presented_while_the_input_channel_remains_open() {
        let (output_tx, output_rx) = mpsc::channel(8);
        let (helper_output_tx, helper_output_rx) = output_mailbox();
        let mapper = tokio::spawn(map_output_events(output_rx, helper_output_tx));
        let receiver =
            tokio::task::spawn_blocking(move || (helper_output_rx.recv(), helper_output_rx.recv()));

        output_tx.send(image(6)).await.unwrap();

        let (connected, frame) = tokio::time::timeout(Duration::from_secs(1), receiver)
            .await
            .expect("settled image should be presented before the input channel closes")
            .unwrap();
        assert_eq!(
            connected,
            Some(HelperEvent::Connected {
                width: 1,
                height: 1
            })
        );
        assert_eq!(
            frame,
            Some(HelperEvent::FrameBgraBytes {
                width: 1,
                height: 1,
                bgra: vec![0, 0, 6, 255],
            })
        );

        drop(output_tx);
        mapper.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn pointer_event_does_not_split_an_image_burst() {
        let (output_tx, output_rx) = mpsc::channel(8);
        let (helper_output_tx, helper_output_rx) = output_mailbox();
        let mapper = tokio::spawn(map_output_events(output_rx, helper_output_tx));

        output_tx.send(image(1)).await.unwrap();
        output_tx
            .send(RdpOutputEvent::PointerPosition { x: 4, y: 5 })
            .await
            .unwrap();
        output_tx.send(image(2)).await.unwrap();
        drop(output_tx);

        mapper.await.unwrap().unwrap();
        assert_eq!(
            helper_output_rx.recv(),
            Some(HelperEvent::Connected {
                width: 1,
                height: 1
            })
        );
        assert_eq!(
            helper_output_rx.recv(),
            Some(HelperEvent::CursorPosition { x: 4, y: 5 })
        );
        assert_eq!(
            helper_output_rx.recv(),
            Some(HelperEvent::FrameBgraBytes {
                width: 1,
                height: 1,
                bgra: vec![0, 0, 2, 255],
            })
        );
        assert_eq!(helper_output_rx.recv(), None);
    }

    fn image(red: u8) -> RdpOutputEvent {
        RdpOutputEvent::Image {
            buffer: vec![u32::from_be_bytes([0, red, 0, 0])],
            width: NonZeroU16::new(1).unwrap(),
            height: NonZeroU16::new(1).unwrap(),
        }
    }
}
