use rmk::lighting::Rgb8;

pub const fn limit_channel(channel: u8, ceiling: u8) -> u8 {
    ((channel as u16 * ceiling as u16 + u8::MAX as u16 / 2) / u8::MAX as u16) as u8
}

pub fn frame_visible(frame: &[Rgb8], ceiling: u8) -> bool {
    frame.iter().any(|pixel| {
        limit_channel(pixel.r, ceiling) != 0
            || limit_channel(pixel.g, ceiling) != 0
            || limit_channel(pixel.b, ceiling) != 0
    })
}
