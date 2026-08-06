use super::{SecurityPolicy, VncCredentials, select_security};
use crate::client::auth::SecurityType;

fn credentials(password: Option<&str>) -> VncCredentials {
    VncCredentials {
        username: Some("alice".to_string()),
        password: password.map(str::to_string),
        domain: Some("example".to_string()),
    }
}

#[test]
fn credentials_debug_is_redacted() {
    let debug = format!("{:?}", credentials(Some("super-secret")));

    assert!(!debug.contains("super-secret"));
    assert!(!debug.contains("alice"));
    assert!(!debug.contains("example"));
    assert!(debug.contains("username_present: true"));
    assert!(debug.contains("password_present: true"));
    assert!(debug.contains("domain_present: true"));
}

#[test]
fn auto_prefers_vnc_auth_when_password_is_available() {
    let advertised = [SecurityType::None, SecurityType::VncAuth];

    let selected = select_security(
        &advertised,
        SecurityPolicy::Auto,
        &credentials(Some("secret")),
        false,
    )
    .expect("security type");

    assert_eq!(selected, SecurityType::VncAuth);
}

#[test]
fn auto_uses_none_when_no_password_source_is_available() {
    let advertised = [SecurityType::None, SecurityType::VncAuth];

    let selected = select_security(&advertised, SecurityPolicy::Auto, &credentials(None), false)
        .expect("security type");

    assert_eq!(selected, SecurityType::None);
}

#[test]
fn auto_supports_servers_offering_only_one_implemented_type() {
    let none = select_security(
        &[SecurityType::None],
        SecurityPolicy::Auto,
        &credentials(Some("unused")),
        false,
    )
    .expect("none");
    let vnc_auth = select_security(
        &[SecurityType::VncAuth],
        SecurityPolicy::Auto,
        &credentials(Some("secret")),
        false,
    )
    .expect("vnc auth");

    assert_eq!(none, SecurityType::None);
    assert_eq!(vnc_auth, SecurityType::VncAuth);
}

#[test]
fn auto_does_not_downgrade_when_an_unsupported_authenticated_type_is_offered() {
    let advertised = [
        SecurityType::None,
        SecurityType::Tls,
        SecurityType::VeNCrypt,
    ];

    let error = select_security(
        &advertised,
        SecurityPolicy::Auto,
        &credentials(Some("secret")),
        false,
    )
    .expect_err("unsupported authenticated security must not downgrade");
    let message = error.to_string();

    assert!(message.contains("None(1)"));
    assert!(message.contains("TLS(18)"));
    assert!(message.contains("VeNCrypt(19)"));
    assert!(message.contains("VncAuth(2)"));
    assert!(message.contains("policy=Auto"));
}

#[test]
fn unknown_security_type_is_preserved_in_diagnostics() {
    let advertised = [SecurityType::Unknown(99)];

    let error = select_security(&advertised, SecurityPolicy::Auto, &credentials(None), false)
        .expect_err("unknown security type");

    assert!(error.to_string().contains("Unknown(99)"));
}

#[test]
fn unsupported_security_error_retains_policy_and_raw_type_ids() {
    let error = select_security(
        &[SecurityType::RA2, SecurityType::Tls, SecurityType::VeNCrypt],
        SecurityPolicy::Auto,
        &credentials(Some("secret")),
        false,
    )
    .expect_err("unsupported security types");

    match error {
        crate::VncError::SecurityNegotiation {
            policy,
            advertised,
            supported,
            ..
        } => {
            assert_eq!(policy, SecurityPolicy::Auto);
            assert_eq!(advertised, "RA2(5), TLS(18), VeNCrypt(19)");
            assert_eq!(supported, "None(1), VncAuth(2)");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn explicit_vnc_auth_requires_server_support_and_password() {
    let unsupported = select_security(
        &[SecurityType::None],
        SecurityPolicy::VncAuth,
        &credentials(Some("secret")),
        false,
    )
    .expect_err("policy mismatch");
    assert!(unsupported.to_string().contains("policy=VncAuth"));

    let missing_password = select_security(
        &[SecurityType::VncAuth],
        SecurityPolicy::VncAuth,
        &credentials(None),
        false,
    )
    .expect_err("missing password");
    assert!(matches!(missing_password, crate::VncError::NoPassword));
}
