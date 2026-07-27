use std::fmt;

use crate::{PixelFormat, Rect, VncEncoding, VncError, MAX_CUT_TEXT_BYTES};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug)]
pub(super) enum ClientMsg {
    SetPixelFormat(PixelFormat),
    SetEncodings(Vec<VncEncoding>),
    FramebufferUpdateRequest(Rect, u8),
    KeyEvent(u32, bool),
    PointerEvent(u16, u16, u8),
    ClientCutText(Vec<u8>),
}

impl ClientMsg {
    pub(super) async fn write<S>(self, writer: &mut S) -> Result<(), VncError>
    where
        S: AsyncWrite + Unpin,
    {
        match self {
            ClientMsg::SetPixelFormat(pf) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [0]       | message-type |
                // | 3            |              | padding      |
                // | 16           | PIXEL_FORMAT | pixel-format |
                // +--------------+--------------+--------------+
                let mut payload = vec![0_u8, 0, 0, 0];
                payload.extend(<PixelFormat as Into<Vec<u8>>>::into(pf));
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::SetEncodings(encodings) => {
                //  +--------------+--------------+---------------------+
                // | No. of bytes | Type [Value] | Description         |
                // +--------------+--------------+---------------------+
                // | 1            | U8 [2]       | message-type        |
                // | 1            |              | padding             |
                // | 2            | U16          | number-of-encodings |
                // +--------------+--------------+---------------------+

                // This is followed by number-of-encodings repetitions of the following:
                // +--------------+--------------+---------------+
                // | No. of bytes | Type [Value] | Description   |
                // +--------------+--------------+---------------+
                // | 4            | S32          | encoding-type |
                // +--------------+--------------+---------------+
                let mut payload = vec![2, 0];
                payload.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
                for e in encodings {
                    payload.write_u32(e.into()).await?;
                }
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::FramebufferUpdateRequest(rect, incremental) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [3]       | message-type |
                // | 1            | U8           | incremental  |
                // | 2            | U16          | x-position   |
                // | 2            | U16          | y-position   |
                // | 2            | U16          | width        |
                // | 2            | U16          | height       |
                // +--------------+--------------+--------------+
                let mut payload = vec![3, incremental];
                payload.extend_from_slice(&rect.x.to_be_bytes());
                payload.extend_from_slice(&rect.y.to_be_bytes());
                payload.extend_from_slice(&rect.width.to_be_bytes());
                payload.extend_from_slice(&rect.height.to_be_bytes());
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::KeyEvent(keycode, down) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [4]       | message-type |
                // | 1            | U8           | down-flag    |
                // | 2            |              | padding      |
                // | 4            | U32          | key          |
                // +--------------+--------------+--------------+
                let mut payload = vec![4, down as u8, 0, 0];
                payload.write_u32(keycode).await?;
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::PointerEvent(x, y, mask) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [5]       | message-type |
                // | 1            | U8           | button-mask  |
                // | 2            | U16          | x-position   |
                // | 2            | U16          | y-position   |
                // +--------------+--------------+--------------+
                let mut payload = vec![5, mask];
                payload.write_u16(x).await?;
                payload.write_u16(y).await?;
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::ClientCutText(bytes) => {
                //   +--------------+--------------+--------------+
                //   | No. of bytes | Type [Value] | Description  |
                //   +--------------+--------------+--------------+
                //   | 1            | U8 [6]       | message-type |
                //   | 3            |              | padding      |
                //   | 4            | U32          | length       |
                //   | length       | U8 array     | text         |
                //   +--------------+--------------+--------------+
                validate_cut_text_length(bytes.len())?;
                let mut header = [6_u8, 0, 0, 0, 0, 0, 0, 0];
                header[4..].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
                writer.write_all(&header).await?;
                writer.write_all(&bytes).await?;
                Ok(())
            }
        }
    }
}

pub(super) enum ServerMsg {
    FramebufferUpdate(u16),
    // SetColorMapEntries,
    Bell,
    ServerCutText(Vec<u8>),
}

impl ServerMsg {
    pub(super) async fn read<S>(reader: &mut S) -> Result<Self, VncError>
    where
        S: AsyncRead + Unpin,
    {
        let server_msg = reader.read_u8().await?;

        match server_msg {
            0 => {
                // FramebufferUpdate
                //   +--------------+--------------+----------------------+
                //   | No. of bytes | Type [Value] | Description          |
                //   +--------------+--------------+----------------------+
                //   | 1            | U8 [0]       | message-type         |
                //   | 1            |              | padding              |
                //   | 2            | U16          | number-of-rectangles |
                //   +--------------+--------------+----------------------+
                let _padding = reader.read_u8().await?;
                let rects = reader.read_u16().await?;
                Ok(ServerMsg::FramebufferUpdate(rects))
            }
            1 => {
                // SetColorMapEntries
                // +--------------+--------------+------------------+
                // | No. of bytes | Type [Value] | Description      |
                // +--------------+--------------+------------------+
                // | 1            | U8 [1]       | message-type     |
                // | 1            |              | padding          |
                // | 2            | U16          | first-color      |
                // | 2            | U16          | number-of-colors |
                // +--------------+--------------+------------------+
                unimplemented!()
            }
            2 => {
                // Bell
                //   +--------------+--------------+--------------+
                //   | No. of bytes | Type [Value] | Description  |
                //   +--------------+--------------+--------------+
                //   | 1            | U8 [2]       | message-type |
                //   +--------------+--------------+--------------+
                Ok(ServerMsg::Bell)
            }
            3 => {
                // ServerCutText
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [3]       | message-type |
                // | 3            |              | padding      |
                // | 4            | U32          | length       |
                // | length       | U8 array     | text         |
                // +--------------+--------------+--------------+
                let mut padding = [0; 3];
                reader.read_exact(&mut padding).await?;
                let len = reader.read_u32().await? as usize;
                validate_cut_text_length(len)?;
                let mut bytes = vec![0; len];
                reader.read_exact(&mut bytes).await?;
                Ok(Self::ServerCutText(bytes))
            }
            _ => Err(VncError::WrongServerMessage),
        }
    }
}

fn validate_cut_text_length(actual: usize) -> Result<(), VncError> {
    if actual > MAX_CUT_TEXT_BYTES {
        return Err(VncError::ClipboardTooLarge {
            actual,
            maximum: MAX_CUT_TEXT_BYTES,
        });
    }
    Ok(())
}

impl fmt::Debug for ServerMsg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FramebufferUpdate(rects) => formatter
                .debug_tuple("FramebufferUpdate")
                .field(rects)
                .finish(),
            Self::Bell => formatter.write_str("Bell"),
            Self::ServerCutText(bytes) => formatter
                .debug_struct("ServerCutText")
                .field("byte_len", &bytes.len())
                .finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClientMsg, ServerMsg};
    use crate::VncError;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn client_cut_text_preserves_raw_wire_bytes() {
        let (mut writer, mut reader) = tokio::io::duplex(64);

        ClientMsg::ClientCutText(b"caf\xe9".to_vec())
            .write(&mut writer)
            .await
            .expect("raw clipboard payload writes");

        let mut wire = vec![0; 12];
        reader
            .read_exact(&mut wire)
            .await
            .expect("wire payload reads");
        assert_eq!(wire, b"\x06\0\0\0\0\0\0\x04caf\xe9");
    }

    #[tokio::test]
    async fn server_cut_text_preserves_raw_wire_bytes() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        writer
            .write_all(b"\x03\0\0\0\0\0\0\x04caf\xe9")
            .await
            .expect("server payload writes");

        let message = ServerMsg::read(&mut reader)
            .await
            .expect("server payload reads");
        let ServerMsg::ServerCutText(bytes) = message else {
            panic!("expected server clipboard payload");
        };
        assert_eq!(bytes, b"caf\xe9");
    }

    #[tokio::test]
    async fn incoming_cut_text_rejects_oversized_length_before_reading_body() {
        let (mut writer, mut reader) = tokio::io::duplex(64);
        let oversized = (crate::MAX_CUT_TEXT_BYTES + 1) as u32;
        writer
            .write_all(&[
                3,
                0,
                0,
                0,
                (oversized >> 24) as u8,
                (oversized >> 16) as u8,
                (oversized >> 8) as u8,
                oversized as u8,
            ])
            .await
            .expect("server header writes");

        let error = ServerMsg::read(&mut reader)
            .await
            .expect_err("oversized clipboard is rejected");
        assert!(matches!(
            error,
            VncError::ClipboardTooLarge {
                actual,
                maximum: crate::MAX_CUT_TEXT_BYTES,
            } if actual == crate::MAX_CUT_TEXT_BYTES + 1
        ));
    }

    #[tokio::test]
    async fn outgoing_cut_text_rejects_oversized_payload() {
        let (mut writer, _) = tokio::io::duplex(64);
        let bytes = vec![b'x'; crate::MAX_CUT_TEXT_BYTES + 1];

        let error = ClientMsg::ClientCutText(bytes)
            .write(&mut writer)
            .await
            .expect_err("oversized clipboard is rejected");
        assert!(matches!(
            error,
            VncError::ClipboardTooLarge {
                actual,
                maximum: crate::MAX_CUT_TEXT_BYTES,
            } if actual == crate::MAX_CUT_TEXT_BYTES + 1
        ));
    }

    #[test]
    fn server_clipboard_debug_only_reports_payload_length() {
        let debug = format!("{:?}", ServerMsg::ServerCutText(vec![17, 34, 51, 68]));

        assert!(debug.contains("byte_len: 4"));
        assert!(!debug.contains("17"));
        assert!(!debug.contains("34"));
        assert!(!debug.contains("51"));
        assert!(!debug.contains("68"));
    }
}
