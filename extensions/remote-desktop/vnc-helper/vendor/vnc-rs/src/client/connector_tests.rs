use std::{
    future::{Future, Ready, ready},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::{VncConnector, authenticate};
use crate::{SecurityPolicy, VncCredentials, VncEncoding, VncError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn assert_legacy_generic_shape<F>(
    _: &VncConnector<tokio::io::DuplexStream, F>,
    _: &Ready<Result<String, VncError>>,
) where
    F: Future<Output = Result<String, VncError>> + Send + Sync + 'static,
{
}

#[test]
fn preserves_legacy_connector_generic_shape() {
    let (client, _) = tokio::io::duplex(16);
    let auth = ready(Ok("secret".to_string()));
    let connector: VncConnector<_, Ready<Result<String, VncError>>> =
        VncConnector::new(client).set_auth_method(auth);
    let witness = ready(Ok("secret".to_string()));

    assert_legacy_generic_shape(&connector, &witness);
}

#[tokio::test]
async fn vnc_auth_path_invokes_password_callback_once() {
    let (client, mut server) = tokio::io::duplex(128);
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_count_for_future = Arc::clone(&callback_count);
    let mut connector = VncConnector::new(client)
        .set_auth_method(async move {
            callback_count_for_future.fetch_add(1, Ordering::SeqCst);
            Ok("secret".to_string())
        })
        .add_encoding(VncEncoding::Raw);

    let server_task = tokio::spawn(async move {
        server.write_all(&[2, 1, 2]).await.unwrap();
        assert_eq!(server.read_u8().await.unwrap(), 2);
        server.write_all(&[0; 16]).await.unwrap();
        let mut response = [0; 16];
        server.read_exact(&mut response).await.unwrap();
        server.write_u32(0).await.unwrap();
    });

    authenticate(&mut connector).await.expect("authenticate");
    server_task.await.unwrap();
    assert_eq!(callback_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn none_path_does_not_invoke_password_callback() {
    let (client, mut server) = tokio::io::duplex(64);
    let callback_count = Arc::new(AtomicUsize::new(0));
    let callback_count_for_future = Arc::clone(&callback_count);
    let mut connector = VncConnector::new(client)
        .set_auth_method(async move {
            callback_count_for_future.fetch_add(1, Ordering::SeqCst);
            Ok("secret".to_string())
        })
        .set_security_policy(SecurityPolicy::None)
        .add_encoding(VncEncoding::Raw);

    let server_task = tokio::spawn(async move {
        server.write_all(&[1, 1]).await.unwrap();
        assert_eq!(server.read_u8().await.unwrap(), 1);
        server.write_u32(0).await.unwrap();
    });

    authenticate(&mut connector).await.expect("authenticate");
    server_task.await.unwrap();
    assert_eq!(callback_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn structured_credentials_complete_vnc_challenge_response() {
    let (client, mut server) = tokio::io::duplex(128);
    let mut connector = VncConnector::new(client)
        .set_credentials(VncCredentials {
            username: Some("alice".to_string()),
            password: Some("secret".to_string()),
            domain: Some("example".to_string()),
        })
        .add_encoding(VncEncoding::Raw);

    let server_task = tokio::spawn(async move {
        server.write_all(&[1, 2]).await.unwrap();
        assert_eq!(server.read_u8().await.unwrap(), 2);
        server.write_all(&[0; 16]).await.unwrap();
        let mut response = [0; 16];
        server.read_exact(&mut response).await.unwrap();
        server.write_u32(0).await.unwrap();
    });

    authenticate(&mut connector).await.expect("authenticate");
    server_task.await.unwrap();
}
