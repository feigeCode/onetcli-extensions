use ironrdp::cliprdr::backend::ClipboardMessage;
use ironrdp_client::rdp::{RdpInputEvent, RdpInputSender};
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputQueueStatus {
    Sent,
    Full,
}

#[derive(Clone, Debug)]
pub(crate) struct HelperInputSender {
    transport: HelperInputTransport,
}

#[derive(Clone, Debug)]
enum HelperInputTransport {
    Production(RdpInputSender),
    #[cfg(test)]
    Test(mpsc::UnboundedSender<RdpInputEvent>),
}

impl HelperInputSender {
    pub(crate) fn production(sender: RdpInputSender) -> Self {
        Self {
            transport: HelperInputTransport::Production(sender),
        }
    }

    pub(crate) fn try_send(&self, event: RdpInputEvent) -> anyhow::Result<InputQueueStatus> {
        self.try_send_with(|| Some(event))
    }

    pub(crate) fn try_send_with(
        &self,
        build_event: impl FnOnce() -> Option<RdpInputEvent>,
    ) -> anyhow::Result<InputQueueStatus> {
        match &self.transport {
            HelperInputTransport::Production(sender) => match sender.try_reserve() {
                Ok(permit) => {
                    if let Some(event) = build_event() {
                        permit.send(event);
                    }
                    Ok(InputQueueStatus::Sent)
                }
                Err(error) => match error {
                    mpsc::error::TrySendError::Full(()) => Ok(InputQueueStatus::Full),
                    mpsc::error::TrySendError::Closed(()) => {
                        Err(anyhow::anyhow!("RDP input channel closed"))
                    }
                },
            },
            #[cfg(test)]
            HelperInputTransport::Test(sender) => {
                if let Some(event) = build_event() {
                    sender
                        .send(event)
                        .map_err(|_| anyhow::anyhow!("RDP input channel closed"))?;
                }
                Ok(InputQueueStatus::Sent)
            }
        }
    }

    pub(crate) fn send_clipboard(&self, message: ClipboardMessage) -> anyhow::Result<()> {
        match &self.transport {
            HelperInputTransport::Production(sender) => sender
                .send_clipboard(message)
                .map_err(|_| anyhow::anyhow!("RDP clipboard input channel closed")),
            #[cfg(test)]
            HelperInputTransport::Test(sender) => sender
                .send(RdpInputEvent::Clipboard(message))
                .map_err(|_| anyhow::anyhow!("RDP clipboard input channel closed")),
        }
    }

    pub(crate) fn request_graceful_close(&self) -> anyhow::Result<()> {
        match &self.transport {
            HelperInputTransport::Production(sender) => {
                sender.request_graceful_close();
                Ok(())
            }
            #[cfg(test)]
            HelperInputTransport::Test(sender) => sender
                .send(RdpInputEvent::Close)
                .map_err(|_| anyhow::anyhow!("RDP input channel closed during shutdown")),
        }
    }

    #[cfg(test)]
    pub(crate) fn test_channel() -> (Self, mpsc::UnboundedReceiver<RdpInputEvent>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (
            Self {
                transport: HelperInputTransport::Test(sender),
            },
            receiver,
        )
    }
}
