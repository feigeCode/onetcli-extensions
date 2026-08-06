pub(crate) mod des;

use std::fmt;

use super::auth::SecurityType;
use crate::VncError;

const SUPPORTED_SECURITY_TYPES: [SecurityType; 2] = [SecurityType::None, SecurityType::VncAuth];

#[derive(Clone, Default, PartialEq, Eq)]
pub struct VncCredentials {
    pub username: Option<String>,
    pub password: Option<String>,
    pub domain: Option<String>,
}

impl fmt::Debug for VncCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VncCredentials")
            .field("username_present", &self.username.is_some())
            .field("password_present", &self.password.is_some())
            .field("domain_present", &self.domain.is_some())
            .finish()
    }
}

impl VncCredentials {
    pub fn password(password: impl Into<String>) -> Self {
        Self {
            password: Some(password.into()),
            ..Self::default()
        }
    }

    pub(crate) fn has_password(&self) -> bool {
        self.password.is_some()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SecurityPolicy {
    #[default]
    Auto,
    None,
    VncAuth,
}

impl fmt::Display for SecurityPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

pub(super) fn select_security(
    advertised: &[SecurityType],
    policy: SecurityPolicy,
    credentials: &VncCredentials,
    password_callback_available: bool,
) -> Result<SecurityType, VncError> {
    let password_available = credentials.has_password() || password_callback_available;
    match policy {
        SecurityPolicy::Auto => select_automatic(advertised, password_available, policy),
        SecurityPolicy::None => {
            select_explicit(advertised, SecurityType::None, password_available, policy)
        }
        SecurityPolicy::VncAuth => select_explicit(
            advertised,
            SecurityType::VncAuth,
            password_available,
            policy,
        ),
    }
}

fn select_automatic(
    advertised: &[SecurityType],
    password_available: bool,
    policy: SecurityPolicy,
) -> Result<SecurityType, VncError> {
    if password_available && advertised.contains(&SecurityType::VncAuth) {
        return Ok(SecurityType::VncAuth);
    }
    if advertised.contains(&SecurityType::None) {
        if password_available && advertises_unsupported_authentication(advertised) {
            return Err(negotiation_error(
                "refusing unauthenticated downgrade while stronger unsupported security types are advertised",
                advertised,
                policy,
            ));
        }
        return Ok(SecurityType::None);
    }
    if advertised.contains(&SecurityType::VncAuth) {
        return Err(VncError::NoPassword);
    }
    Err(negotiation_error(
        "server did not advertise a supported security type",
        advertised,
        policy,
    ))
}

fn select_explicit(
    advertised: &[SecurityType],
    requested: SecurityType,
    password_available: bool,
    policy: SecurityPolicy,
) -> Result<SecurityType, VncError> {
    if !advertised.contains(&requested) {
        return Err(negotiation_error(
            "server did not advertise the security type required by policy",
            advertised,
            policy,
        ));
    }
    if requested == SecurityType::VncAuth && !password_available {
        return Err(VncError::NoPassword);
    }
    Ok(requested)
}

fn advertises_unsupported_authentication(advertised: &[SecurityType]) -> bool {
    advertised.iter().any(|security_type| {
        security_type.is_authenticated() && !SUPPORTED_SECURITY_TYPES.contains(security_type)
    })
}

fn negotiation_error(
    reason: &str,
    advertised: &[SecurityType],
    policy: SecurityPolicy,
) -> VncError {
    VncError::SecurityNegotiation {
        reason: reason.to_string(),
        policy,
        advertised: describe_security_types(advertised),
        supported: describe_security_types(&SUPPORTED_SECURITY_TYPES),
    }
}

fn describe_security_types(security_types: &[SecurityType]) -> String {
    security_types
        .iter()
        .map(|security_type| security_type.describe())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;
