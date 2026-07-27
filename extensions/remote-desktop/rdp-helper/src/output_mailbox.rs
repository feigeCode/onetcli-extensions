use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::protocol::HelperEvent;

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
    control: VecDeque<HelperEvent>,
    latest_frame: Option<HelperEvent>,
    latest_delta: Option<HelperEvent>,
    sender_count: usize,
    receiver_alive: bool,
}

pub fn output_mailbox() -> (OutputSender, OutputReceiver) {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            control: VecDeque::new(),
            latest_frame: None,
            latest_delta: None,
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
    pub fn send(&self, event: HelperEvent) -> Result<(), MailboxClosed> {
        let mut state = lock(&self.shared);
        if !state.receiver_alive {
            return Err(MailboxClosed);
        }
        match event {
            frame @ HelperEvent::FrameBgraBytes { .. } => {
                state.latest_frame = Some(frame);
                state.latest_delta = None;
            }
            delta @ HelperEvent::FrameBgraRects { .. } => {
                state.latest_delta = Some(match state.latest_delta.take() {
                    Some(previous) => merge_deltas(previous, delta),
                    None => delta,
                });
            }
            terminal @ (HelperEvent::ConnectionFailure { .. } | HelperEvent::Terminated { .. }) => {
                state.latest_frame = None;
                state.latest_delta = None;
                discard_pending_cursor_events(&mut state.control);
                state.control.push_back(terminal);
            }
            reconnecting @ HelperEvent::Reconnecting { .. } => {
                discard_pending_cursor_events(&mut state.control);
                state.control.push_back(reconnecting);
            }
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
    pub fn recv(&self) -> Option<HelperEvent> {
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

fn merge_deltas(previous: HelperEvent, next: HelperEvent) -> HelperEvent {
    match (previous, next) {
        (
            HelperEvent::FrameBgraRects {
                width,
                height,
                mut rects,
                mut bgra,
            },
            HelperEvent::FrameBgraRects {
                width: next_width,
                height: next_height,
                rects: next_rects,
                bgra: next_bgra,
            },
        ) if width == next_width && height == next_height => {
            rects.extend(next_rects);
            bgra.extend(next_bgra);
            HelperEvent::FrameBgraRects {
                width,
                height,
                rects,
                bgra,
            }
        }
        (_, next) => next,
    }
}

fn enqueue_control(control: &mut VecDeque<HelperEvent>, event: HelperEvent) {
    match (control.back_mut(), event) {
        (
            Some(HelperEvent::CursorPosition { x, y }),
            HelperEvent::CursorPosition {
                x: next_x,
                y: next_y,
            },
        ) => {
            *x = next_x;
            *y = next_y;
        }
        (
            Some(previous @ HelperEvent::CursorRgbaBytes { .. }),
            next @ HelperEvent::CursorRgbaBytes { .. },
        ) => *previous = next,
        (_, event) => control.push_back(event),
    }
}

fn discard_pending_cursor_events(control: &mut VecDeque<HelperEvent>) {
    control.retain(|event| {
        !matches!(
            event,
            HelperEvent::CursorDefault
                | HelperEvent::CursorHidden
                | HelperEvent::CursorPosition { .. }
                | HelperEvent::CursorRgbaBytes { .. }
        )
    });
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
        formatter.write_str("RDP helper output mailbox is closed")
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
