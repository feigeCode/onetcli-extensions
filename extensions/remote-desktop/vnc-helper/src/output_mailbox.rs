use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::runtime::RemoteDesktopOutput;

pub struct OutputSender {
    shared: Arc<Shared>,
}

pub struct OutputReceiver {
    shared: Arc<Shared>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MailboxClosed;

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

struct State {
    control: VecDeque<RemoteDesktopOutput>,
    latest_frame: Option<RemoteDesktopOutput>,
    latest_delta: Option<RemoteDesktopOutput>,
    accepting_session_outputs: bool,
    sender_count: usize,
    receiver_alive: bool,
}

pub fn output_mailbox() -> (OutputSender, OutputReceiver) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            control: VecDeque::new(),
            latest_frame: None,
            latest_delta: None,
            accepting_session_outputs: true,
            sender_count: 1,
            receiver_alive: true,
        }),
        ready: Condvar::new(),
    });
    (
        OutputSender {
            shared: shared.clone(),
        },
        OutputReceiver { shared },
    )
}

impl OutputSender {
    pub fn begin_generation(&self) {
        let mut state = lock(&self.shared);
        close_session_outputs(&mut state);
        drop(state);
        self.shared.ready.notify_one();
    }

    pub fn send(&self, output: RemoteDesktopOutput) -> Result<(), MailboxClosed> {
        let mut state = lock(&self.shared);
        if !state.receiver_alive {
            return Err(MailboxClosed);
        }
        match output {
            RemoteDesktopOutput::Reconnecting(reconnect) => {
                close_session_outputs(&mut state);
                state
                    .control
                    .push_back(RemoteDesktopOutput::Reconnecting(reconnect));
            }
            connected @ RemoteDesktopOutput::Connected { .. } => {
                state.accepting_session_outputs = true;
                state.control.push_back(connected);
            }
            frame @ RemoteDesktopOutput::Frame { .. } if state.accepting_session_outputs => {
                state.latest_frame = Some(frame);
                state.latest_delta = None;
            }
            delta @ RemoteDesktopOutput::FrameBgraRects { .. }
                if state.accepting_session_outputs =>
            {
                state.latest_delta = Some(match state.latest_delta.take() {
                    Some(previous) => merge_deltas(previous, delta),
                    None => delta,
                });
            }
            RemoteDesktopOutput::Frame { .. } | RemoteDesktopOutput::FrameBgraRects { .. } => {}
            terminal @ (RemoteDesktopOutput::ConnectionFailure(_)
            | RemoteDesktopOutput::Terminated(_)) => {
                close_session_outputs(&mut state);
                state.control.push_back(terminal);
            }
            control if is_session_scoped_control(&control) && !state.accepting_session_outputs => {}
            control => enqueue_control(&mut state.control, control),
        }
        drop(state);
        self.shared.ready.notify_one();
        Ok(())
    }
}

impl Clone for OutputSender {
    fn clone(&self) -> Self {
        lock(&self.shared).sender_count += 1;
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl Drop for OutputSender {
    fn drop(&mut self) {
        let mut state = lock(&self.shared);
        state.sender_count = state.sender_count.saturating_sub(1);
        let closed = state.sender_count == 0;
        drop(state);
        if closed {
            self.shared.ready.notify_all();
        }
    }
}

impl OutputReceiver {
    pub fn recv(&self) -> Option<RemoteDesktopOutput> {
        let mut state = lock(&self.shared);
        loop {
            if let Some(control) = state.control.pop_front() {
                return Some(control);
            }
            if let Some(frame) = state.latest_frame.take() {
                return Some(frame);
            }
            if let Some(delta) = state.latest_delta.take() {
                return Some(delta);
            }
            if state.sender_count == 0 {
                return None;
            }
            state = self
                .shared
                .ready
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

impl Drop for OutputReceiver {
    fn drop(&mut self) {
        let mut state = lock(&self.shared);
        state.receiver_alive = false;
        state.control.clear();
        state.latest_frame = None;
        state.latest_delta = None;
        drop(state);
        self.shared.ready.notify_all();
    }
}

fn merge_deltas(previous: RemoteDesktopOutput, next: RemoteDesktopOutput) -> RemoteDesktopOutput {
    match (previous, next) {
        (
            RemoteDesktopOutput::FrameBgraRects {
                width,
                height,
                mut rects,
                mut bgra,
            },
            RemoteDesktopOutput::FrameBgraRects {
                width: next_width,
                height: next_height,
                rects: next_rects,
                bgra: next_bgra,
            },
        ) if width == next_width && height == next_height => {
            rects.extend(next_rects);
            bgra.extend(next_bgra);
            RemoteDesktopOutput::FrameBgraRects {
                width,
                height,
                rects,
                bgra,
            }
        }
        (_, next) => next,
    }
}

fn enqueue_control(control: &mut VecDeque<RemoteDesktopOutput>, output: RemoteDesktopOutput) {
    match (control.back_mut(), output) {
        (
            Some(RemoteDesktopOutput::CursorPosition { x, y }),
            RemoteDesktopOutput::CursorPosition {
                x: next_x,
                y: next_y,
            },
        ) => {
            *x = next_x;
            *y = next_y;
        }
        (
            Some(previous @ RemoteDesktopOutput::CursorBitmap(_)),
            next @ RemoteDesktopOutput::CursorBitmap(_),
        ) => *previous = next,
        (_, output) => control.push_back(output),
    }
}

fn close_session_outputs(state: &mut State) {
    state.latest_frame = None;
    state.latest_delta = None;
    state.accepting_session_outputs = false;
    discard_pending_session_controls(&mut state.control);
}

fn discard_pending_session_controls(control: &mut VecDeque<RemoteDesktopOutput>) {
    control.retain(|output| !is_session_scoped_control(output));
}

fn is_session_scoped_control(output: &RemoteDesktopOutput) -> bool {
    matches!(
        output,
        RemoteDesktopOutput::ClipboardText { .. }
            | RemoteDesktopOutput::CursorDefault
            | RemoteDesktopOutput::CursorHidden
            | RemoteDesktopOutput::CursorPosition { .. }
            | RemoteDesktopOutput::CursorBitmap(_)
    )
}

impl fmt::Debug for OutputSender {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("OutputSender").finish()
    }
}

impl fmt::Debug for OutputReceiver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("OutputReceiver").finish()
    }
}

impl fmt::Display for MailboxClosed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VNC helper output mailbox is closed")
    }
}

impl std::error::Error for MailboxClosed {}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
#[path = "output_mailbox_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "output_mailbox_cursor_tests.rs"]
mod cursor_tests;
