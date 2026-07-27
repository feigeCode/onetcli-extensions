use ironrdp::connector::{self, Credentials};
use ironrdp::pdu::rdp::capability_sets::{MajorPlatformType, client_codecs_capabilities};
use ironrdp::pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_client::config::{ClipboardType, Config, Destination};

use crate::protocol::ConnectRequest;

pub(super) fn build_config(connect: ConnectRequest) -> anyhow::Result<Config> {
    let codecs = client_codecs_capabilities(&[])
        .map_err(|help| anyhow::anyhow!("failed to build bitmap codec capabilities: {help}"))?;
    let connector = connector::Config {
        credentials: Credentials::UsernamePassword {
            username: connect.username.unwrap_or_default(),
            password: connect.password.unwrap_or_default(),
        },
        domain: connect.domain,
        enable_tls: true,
        enable_credssp: true,
        desktop_size: connector::DesktopSize {
            width: connect.width,
            height: connect.height,
        },
        desktop_scale_factor: connect.scale_factor,
        keyboard_type: ironrdp::pdu::gcc::KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        bitmap: Some(connector::BitmapConfig {
            lossy_compression: true,
            color_depth: 32,
            codecs,
        }),
        dig_product_id: String::new(),
        client_build: client_build()?,
        client_name: whoami::fallible::hostname().unwrap_or_else(|_| "navop-rdp".to_string()),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_string(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        platform: platform_type(),
        hardware_id: None,
        license_cache: None,
        request_data: None,
        autologon: true,
        enable_audio_playback: connect.audio_playback,
        enable_server_pointer: true,
        pointer_software_rendering: false,
        multitransport_flags: None,
        // Do not advertise bulk compression until IronRDP can safely carry its
        // decompressor state across Deactivation-Reactivation. Reusing that
        // history currently corrupts some FastPath bitmap updates after a
        // resize/reconnect and disconnects the session with a decode error.
        compression_type: None,
        performance_flags: PerformanceFlags::default(),
        timezone_info: TimezoneInfo::default(),
    };

    Ok(Config {
        log_file: None,
        gw: None,
        kerberos_config: None,
        destination: connect.destination.parse::<Destination>()?,
        connector,
        clipboard_type: ClipboardType::Enable,
        rdcleanpath: None,
        fake_events_interval: None,
        dvc_pipe_proxies: Vec::new(),
        #[cfg(windows)]
        dvc_plugins: Vec::new(),
    })
}

fn client_build() -> anyhow::Result<u32> {
    let version = semver::Version::parse(env!("CARGO_PKG_VERSION"))?;
    Ok((version.major * 100 + version.minor * 10 + version.patch).try_into()?)
}

fn platform_type() -> MajorPlatformType {
    match whoami::platform() {
        whoami::Platform::Windows => MajorPlatformType::WINDOWS,
        whoami::Platform::Linux => MajorPlatformType::UNIX,
        whoami::Platform::MacOS => MajorPlatformType::MACINTOSH,
        whoami::Platform::Ios => MajorPlatformType::IOS,
        whoami::Platform::Android => MajorPlatformType::ANDROID,
        _ => MajorPlatformType::UNSPECIFIED,
    }
}

#[cfg(test)]
mod tests {
    use ironrdp::pdu::rdp::client_info::PerformanceFlags;

    use super::*;

    #[test]
    fn matches_ironrdp_viewer_performance_flags() {
        let config = build_config(ConnectRequest {
            destination: "127.0.0.1:3389".to_string(),
            username: None,
            password: None,
            domain: None,
            width: 1280,
            height: 720,
            scale_factor: 200,
            audio_playback: false,
            audio_capture: false,
            shared_folders: Vec::new(),
        })
        .expect("config builds");

        let flags = config.connector.performance_flags;
        assert_eq!(200, config.connector.desktop_scale_factor);
        assert_eq!(PerformanceFlags::default(), flags);
        assert!(!flags.contains(PerformanceFlags::DISABLE_THEMING));
        assert!(!flags.contains(PerformanceFlags::ENABLE_DESKTOP_COMPOSITION));
    }

    #[test]
    fn applies_requested_audio_playback_setting() {
        for audio_playback in [false, true] {
            let config = build_config(ConnectRequest {
                destination: "127.0.0.1:3389".to_string(),
                username: None,
                password: None,
                domain: None,
                width: 1280,
                height: 720,
                scale_factor: 100,
                audio_playback,
                audio_capture: false,
                shared_folders: Vec::new(),
            })
            .expect("config builds");

            assert_eq!(
                audio_playback, config.connector.enable_audio_playback,
                "connector audio playback must follow the Connect request"
            );
        }
    }

    #[test]
    fn does_not_advertise_bulk_compression() {
        let config = build_config(ConnectRequest {
            destination: "127.0.0.1:3389".to_string(),
            username: None,
            password: None,
            domain: None,
            width: 1280,
            height: 720,
            scale_factor: 100,
            audio_playback: false,
            audio_capture: false,
            shared_folders: Vec::new(),
        })
        .expect("config builds");

        assert_eq!(None, config.connector.compression_type);
    }
}
