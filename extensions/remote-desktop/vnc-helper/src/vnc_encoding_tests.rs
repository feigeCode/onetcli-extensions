use super::*;
use crate::output_mailbox::output_mailbox;

#[test]
fn waits_while_incremental_refresh_is_in_flight() {
    let now = Instant::now();

    assert_eq!(
        RefreshAction::Wait,
        refresh_action(
            now,
            now - Duration::from_secs(1),
            now - Duration::from_secs(1),
            Some(now - Duration::from_millis(100)),
            None,
        )
    );
}

#[test]
fn resumes_incremental_refresh_after_empty_update_grace_period() {
    let now = Instant::now();

    assert_eq!(
        RefreshAction::Incremental,
        refresh_action(
            now,
            now - Duration::from_secs(1),
            now - Duration::from_secs(1),
            Some(now - VNC_REFRESH_RESPONSE_TIMEOUT),
            None,
        )
    );
}

#[test]
fn probes_idle_connection_with_non_incremental_refresh() {
    let now = Instant::now();

    assert_eq!(
        RefreshAction::LivenessProbe,
        refresh_action(
            now,
            now - Duration::from_secs(1),
            now - VNC_LIVENESS_TIMEOUT,
            None,
            None,
        )
    );
}

#[test]
fn reconnects_when_liveness_probe_has_no_response() {
    let now = Instant::now();

    assert_eq!(
        RefreshAction::Reconnect,
        refresh_action(
            now,
            now - Duration::from_secs(1),
            now - Duration::from_secs(30),
            Some(now - VNC_LIVENESS_PROBE_TIMEOUT),
            Some(now - VNC_LIVENESS_PROBE_TIMEOUT),
        )
    );
}

#[test]
fn burst_frames_keep_only_latest_pending_output() {
    let (output_tx, output_rx) = output_mailbox();
    let mut framebuffer = VncFramebufferState::default();
    framebuffer.set_resolution(
        vnc_client::Screen {
            width: 1,
            height: 1,
        },
        &output_tx,
    );
    for value in [1, 2, 3] {
        framebuffer
            .patch_rect(
                Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                &[value, 0, 0, 255],
            )
            .unwrap();
        framebuffer.flush_frame(&output_tx);
    }

    assert!(matches!(
        output_rx.recv(),
        Some(RemoteDesktopOutput::Connected { .. })
    ));
    let pending = output_rx.recv();
    assert_eq!(
        Some(RemoteDesktopOutput::Frame {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 1, 255],
        }),
        pending
    );
    assert!(matches!(
        output_rx.recv(),
        Some(RemoteDesktopOutput::FrameBgraRects { .. })
    ));
    drop(output_tx);
    assert_eq!(None, output_rx.recv());
}

#[test]
fn coalesces_multiple_rectangles_into_one_flushed_frame() {
    let (output_tx, output_rx) = output_mailbox();
    let mut framebuffer = VncFramebufferState::default();
    framebuffer.set_resolution(
        vnc_client::Screen {
            width: 2,
            height: 1,
        },
        &output_tx,
    );

    framebuffer
        .patch_rect(
            Rect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            &[255, 0, 0, 255],
        )
        .unwrap();
    framebuffer
        .patch_rect(
            Rect {
                x: 1,
                y: 0,
                width: 1,
                height: 1,
            },
            &[0, 0, 255, 255],
        )
        .unwrap();

    assert_eq!(
        output_rx.recv().unwrap(),
        RemoteDesktopOutput::Connected {
            width: 2,
            height: 1,
            capabilities: vnc_capabilities(),
        }
    );
    framebuffer.flush_frame(&output_tx);

    assert_eq!(
        output_rx.recv().unwrap(),
        RemoteDesktopOutput::Frame {
            width: 2,
            height: 1,
            rgba: vec![0, 0, 255, 255, 255, 0, 0, 255],
        }
    );
}

#[test]
fn resolution_does_not_emit_black_frame_before_first_real_update() {
    let (output_tx, output_rx) = output_mailbox();
    let mut framebuffer = VncFramebufferState::default();
    framebuffer.set_resolution(
        vnc_client::Screen {
            width: 2,
            height: 1,
        },
        &output_tx,
    );

    assert!(matches!(
        output_rx.recv(),
        Some(RemoteDesktopOutput::Connected { .. })
    ));
    framebuffer.flush_frame(&output_tx);
    drop(output_tx);
    assert_eq!(None, output_rx.recv());
}
