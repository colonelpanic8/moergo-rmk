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

pub const fn chain_should_power(
    usb_powered: bool,
    sleeping: bool,
    frame_visible: bool,
    keep_power_while_awake: bool,
    keep_power_while_suspended: bool,
) -> bool {
    if sleeping {
        keep_power_while_suspended
    } else {
        keep_power_while_awake || usb_powered || frame_visible
    }
}
