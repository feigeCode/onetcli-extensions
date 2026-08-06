use super::security;
use crate::{VncError, VncVersion};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const MAX_SECURITY_REASON_BYTES: usize = 64 * 1024;

pub(super) async fn read_failure_reason<S>(reader: &mut S) -> Result<String, VncError>
where
    S: AsyncRead + Unpin,
{
    let reason_len = reader.read_u32().await? as usize;
    if reason_len > MAX_SECURITY_REASON_BYTES {
        return Err(VncError::General(format!(
            "VNC security failure has an oversized reason ({reason_len} bytes)"
        )));
    }
    let mut reason = vec![0; reason_len];
    reader.read_exact(&mut reason).await?;
    Ok(String::from_utf8_lossy(&reason).into_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SecurityType {
    Invalid,
    None,
    VncAuth,
    RA2,
    RA2ne,
    Tight,
    Ultra,
    Tls,
    VeNCrypt,
    GtkVncSasl,
    Md5Hash,
    ColinDeanXvp,
    Unknown(u8),
}

impl From<u8> for SecurityType {
    fn from(id: u8) -> Self {
        match id {
            0 => Self::Invalid,
            1 => Self::None,
            2 => Self::VncAuth,
            5 => Self::RA2,
            6 => Self::RA2ne,
            16 => Self::Tight,
            17 => Self::Ultra,
            18 => Self::Tls,
            19 => Self::VeNCrypt,
            20 => Self::GtkVncSasl,
            21 => Self::Md5Hash,
            22 => Self::ColinDeanXvp,
            unknown => Self::Unknown(unknown),
        }
    }
}

impl From<SecurityType> for u8 {
    fn from(security_type: SecurityType) -> Self {
        security_type.id()
    }
}

impl SecurityType {
    pub(super) const fn id(self) -> u8 {
        match self {
            Self::Invalid => 0,
            Self::None => 1,
            Self::VncAuth => 2,
            Self::RA2 => 5,
            Self::RA2ne => 6,
            Self::Tight => 16,
            Self::Ultra => 17,
            Self::Tls => 18,
            Self::VeNCrypt => 19,
            Self::GtkVncSasl => 20,
            Self::Md5Hash => 21,
            Self::ColinDeanXvp => 22,
            Self::Unknown(id) => id,
        }
    }

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Invalid => "Invalid",
            Self::None => "None",
            Self::VncAuth => "VncAuth",
            Self::RA2 => "RA2",
            Self::RA2ne => "RA2ne",
            Self::Tight => "Tight",
            Self::Ultra => "Ultra",
            Self::Tls => "TLS",
            Self::VeNCrypt => "VeNCrypt",
            Self::GtkVncSasl => "GtkVncSasl",
            Self::Md5Hash => "Md5Hash",
            Self::ColinDeanXvp => "ColinDeanXvp",
            Self::Unknown(_) => "Unknown",
        }
    }

    pub(super) fn describe(self) -> String {
        format!("{}({})", self.name(), self.id())
    }

    pub(super) const fn is_authenticated(self) -> bool {
        !matches!(self, Self::Invalid | Self::None)
    }

    pub(super) async fn read<S>(reader: &mut S, version: &VncVersion) -> Result<Vec<Self>, VncError>
    where
        S: AsyncRead + Unpin,
    {
        match version {
            VncVersion::RFB33 => {
                let raw_security_type = reader.read_u32().await?;
                let raw_security_type = u8::try_from(raw_security_type).map_err(|_| {
                    VncError::General(format!(
                        "RFB 3.3 security type {raw_security_type} exceeds one byte"
                    ))
                })?;
                let security_type = raw_security_type.into();
                if let SecurityType::Invalid = security_type {
                    return Err(VncError::General(read_failure_reason(reader).await?));
                }
                Ok(vec![security_type])
            }
            _ => {
                // +--------------------------+-------------+--------------------------+
                // | No. of bytes             | Type        | Description              |
                // |                          | [Value]     |                          |
                // +--------------------------+-------------+--------------------------+
                // | 1                        | U8          | number-of-security-types |
                // | number-of-security-types | U8 array    | security-types           |
                // +--------------------------+-------------+--------------------------+
                let num = reader.read_u8().await?;

                if num == 0 {
                    return Err(VncError::General(read_failure_reason(reader).await?));
                }
                let mut sec_types = vec![];
                for _ in 0..num {
                    sec_types.push(reader.read_u8().await?.into());
                }
                tracing::trace!("Server supported security type: {:?}", sec_types);
                Ok(sec_types)
            }
        }
    }

    pub(super) async fn write<S>(&self, writer: &mut S) -> Result<(), VncError>
    where
        S: AsyncWrite + Unpin,
    {
        writer.write_all(&[(*self).into()]).await?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub(super) enum AuthResult {
    Ok = 0,
    Failed = 1,
}

impl AuthResult {
    pub(super) fn decode(raw: u32) -> Result<Self, VncError> {
        match raw {
            0 => Ok(Self::Ok),
            1 => Ok(Self::Failed),
            invalid => Err(VncError::InvalidAuthResult(invalid)),
        }
    }
}

impl From<AuthResult> for u32 {
    fn from(e: AuthResult) -> Self {
        e as u32
    }
}

pub(super) struct AuthHelper {
    challenge: [u8; 16],
    key: [u8; 8],
}

impl AuthHelper {
    pub(super) async fn read<S>(reader: &mut S, credential: &str) -> Result<Self, VncError>
    where
        S: AsyncRead + Unpin,
    {
        let mut challenge = [0; 16];
        reader.read_exact(&mut challenge).await?;

        let credential_len = credential.len();
        let mut key = [0u8; 8];
        for (i, key_i) in key.iter_mut().enumerate() {
            let c = if i < credential_len {
                credential.as_bytes()[i]
            } else {
                0
            };
            let mut cs = 0u8;
            for j in 0..8 {
                cs |= ((c >> j) & 1) << (7 - j)
            }
            *key_i = cs;
        }

        Ok(Self { challenge, key })
    }

    pub(super) async fn write<S>(&self, writer: &mut S) -> Result<(), VncError>
    where
        S: AsyncWrite + Unpin,
    {
        let encrypted = security::des::encrypt(&self.challenge, &self.key);
        writer.write_all(&encrypted).await?;
        Ok(())
    }

    pub(super) async fn finish<S>(self, reader: &mut S) -> Result<AuthResult, VncError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let result = reader.read_u32().await?;
        AuthResult::decode(result)
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
