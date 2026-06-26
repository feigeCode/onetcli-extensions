pub fn rdp_u32_pixels_to_bgra(buffer: &[u32]) -> Vec<u8> {
    let mut bgra = Vec::with_capacity(buffer.len() * 4);

    for pixel in buffer {
        let [_, r, g, b] = pixel.to_be_bytes();
        bgra.extend_from_slice(&[b, g, r, 0xff]);
    }

    bgra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_ironrdp_pixels_to_bgra_for_gpui() {
        assert_eq!(
            rdp_u32_pixels_to_bgra(&[0x00112233, 0x00abcdef]),
            vec![0x33, 0x22, 0x11, 0xff, 0xef, 0xcd, 0xab, 0xff]
        );
    }
}
