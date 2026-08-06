use std::time::Duration;

use tokio::io::AsyncWriteExt;

use super::{AuthResult, SecurityType};
use crate::{VncError, VncVersion};

#[test]
fn rejects_unknown_auth_result_without_unsafe_enum_conversion() {
    let error = AuthResult::decode(7).expect_err("invalid auth result");

    assert!(matches!(error, VncError::InvalidAuthResult(7)));
}

#[tokio::test]
async fn reads_declared_security_failure_reason_without_waiting_for_eof() {
    let (mut client, mut server) = tokio::io::duplex(128);
    let server_task = tokio::spawn(async move {
        server.write_u8(0).await.unwrap();
        server.write_u32(6).await.unwrap();
        server.write_all(b"denied").await.unwrap();
        tokio::time::sleep(Duration::from_secs(1)).await;
    });

    let result = tokio::time::timeout(
        Duration::from_millis(100),
        SecurityType::read(&mut client, &VncVersion::RFB38),
    )
    .await
    .expect("security failure reason must not wait for EOF");

    assert!(matches!(result, Err(VncError::General(reason)) if reason == "denied"));
    server_task.abort();
}

#[tokio::test]
async fn rejects_oversized_security_failure_reason_before_allocating_payload() {
    let (mut client, mut server) = tokio::io::duplex(16);
    server.write_u8(0).await.unwrap();
    server.write_u32(64 * 1024 + 1).await.unwrap();

    let error = SecurityType::read(&mut client, &VncVersion::RFB38)
        .await
        .expect_err("oversized failure reason");

    assert!(error.to_string().contains("oversized reason"));
}
