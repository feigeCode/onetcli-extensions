//! Read-only Elasticsearch universal resource provider.
//!
//! The provider owns only transport translation. Elasticsearch permissions
//! remain host-authoritative: connection endpoints are checked by the host
//! before `resource/open`, and API keys are resolved through the reverse Host
//! API after the extension manifest's `secrets:read:*` permission is checked.

mod client;
mod config;
mod error;
mod ipc;
mod server;
mod state;

#[tokio::main]
async fn main() {
    server::run().await;
}
