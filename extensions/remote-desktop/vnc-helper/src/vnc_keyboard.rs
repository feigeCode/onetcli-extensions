use crate::runtime::RemoteKey;

pub fn remote_key_to_keysym(key: &RemoteKey) -> Option<u32> {
    match key {
        RemoteKey::KeySym(value) => *value,
        RemoteKey::Scancode(value) => scancode_to_keysym(*value)?,
    }
    .into()
}

fn scancode_to_keysym(scancode: u16) -> Option<u32> {
    Some(match scancode {
        0x01 => 0xff1b,
        0x02 => b'1' as u32,
        0x03 => b'2' as u32,
        0x04 => b'3' as u32,
        0x05 => b'4' as u32,
        0x06 => b'5' as u32,
        0x07 => b'6' as u32,
        0x08 => b'7' as u32,
        0x09 => b'8' as u32,
        0x0a => b'9' as u32,
        0x0b => b'0' as u32,
        0x0e => 0xff08,
        0x0f => 0xff09,
        0x10 => b'q' as u32,
        0x11 => b'w' as u32,
        0x12 => b'e' as u32,
        0x13 => b'r' as u32,
        0x14 => b't' as u32,
        0x15 => b'y' as u32,
        0x16 => b'u' as u32,
        0x17 => b'i' as u32,
        0x18 => b'o' as u32,
        0x19 => b'p' as u32,
        0x1c => 0xff0d,
        0x1d => 0xffe3,
        0x1e => b'a' as u32,
        0x1f => b's' as u32,
        0x20 => b'd' as u32,
        0x21 => b'f' as u32,
        0x22 => b'g' as u32,
        0x23 => b'h' as u32,
        0x24 => b'j' as u32,
        0x25 => b'k' as u32,
        0x26 => b'l' as u32,
        0x2a => 0xffe1,
        0x2c => b'z' as u32,
        0x2d => b'x' as u32,
        0x2e => b'c' as u32,
        0x2f => b'v' as u32,
        0x30 => b'b' as u32,
        0x31 => b'n' as u32,
        0x32 => b'm' as u32,
        0x38 => 0xffe9,
        0x39 => 0x20,
        0x3a => 0xffe5,
        0x3b..=0x44 => 0xffbe + (scancode as u32 - 0x3b),
        0x57 => 0xffc8,
        0x58 => 0xffc9,
        0xe01c => 0xff0d,
        0xe01d => 0xffe4,
        0xe038 => 0xffea,
        0xe047 => 0xff50,
        0xe048 => 0xff52,
        0xe049 => 0xff55,
        0xe04b => 0xff51,
        0xe04d => 0xff53,
        0xe04f => 0xff57,
        0xe050 => 0xff54,
        0xe051 => 0xff56,
        0xe052 => 0xff63,
        0xe053 => 0xffff,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_scancodes_to_x11_keysyms() {
        assert_eq!(
            remote_key_to_keysym(&RemoteKey::Scancode(0x1c)),
            Some(0xff0d)
        );
        assert_eq!(
            remote_key_to_keysym(&RemoteKey::Scancode(0xe048)),
            Some(0xff52)
        );
        assert_eq!(
            remote_key_to_keysym(&RemoteKey::Scancode(0x3f)),
            Some(0xffc2)
        );
    }

    #[test]
    fn maps_direct_keysym() {
        assert_eq!(remote_key_to_keysym(&RemoteKey::KeySym(0x61)), Some(0x61));
    }
}
