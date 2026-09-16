use rmk::lighting::Rgb8;

/// sRGB decode tabulated to 8-bit LED duty.
///
/// A WS2812 channel is linear in duty; an sRGB hex byte is not. Writing the
/// byte straight to the chain therefore emits far more light than the same
/// byte does on a display, and because each channel is wrong by its own
/// factor it also moves the hue: `#ff8000` leaves an encoder at G/R = 0.22
/// and reaches the LED at G/R = 0.50, so on-screen orange arrives yellow.
#[rustfmt::skip]
const SRGB_DECODE: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3,
    4, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7,
    8, 8, 8, 8, 9, 9, 9, 10, 10, 10, 11, 11, 12, 12, 12, 13,
    13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 17, 18, 18, 19, 19, 20,
    20, 21, 22, 22, 23, 23, 24, 24, 25, 25, 26, 27, 27, 28, 29, 29,
    30, 30, 31, 32, 32, 33, 34, 35, 35, 36, 37, 37, 38, 39, 40, 41,
    41, 42, 43, 44, 45, 45, 46, 47, 48, 49, 50, 51, 51, 52, 53, 54,
    55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70,
    71, 72, 73, 74, 76, 77, 78, 79, 80, 81, 82, 84, 85, 86, 87, 88,
    90, 91, 92, 93, 95, 96, 97, 99, 100, 101, 103, 104, 105, 107, 108, 109,
    111, 112, 114, 115, 116, 118, 119, 121, 122, 124, 125, 127, 128, 130, 131, 133,
    134, 136, 138, 139, 141, 142, 144, 146, 147, 149, 151, 152, 154, 156, 157, 159,
    161, 163, 164, 166, 168, 170, 171, 173, 175, 177, 179, 181, 183, 184, 186, 188,
    190, 192, 194, 196, 198, 200, 202, 204, 206, 208, 210, 212, 214, 216, 218, 220,
    222, 224, 226, 229, 231, 233, 235, 237, 239, 242, 244, 246, 248, 250, 253, 255,
];

/// How a board's chain turns a requested colour into LED duty.
///
/// [`ColorProfile::LINEAR`] is the historical behaviour, and every stored
/// configuration was authored by eye against it. Changing a board's profile
/// therefore changes how every existing config on that board looks; it is a
/// board-level decision, not an incidental one.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ColorProfile {
    decode: bool,
    /// Per-channel white-point trim. 255 leaves a channel untouched.
    trim: Rgb8,
}

impl ColorProfile {
    const NO_TRIM: Rgb8 = Rgb8::new(u8::MAX, u8::MAX, u8::MAX);

    /// The requested byte is the duty.
    pub const LINEAR: Self = Self {
        decode: false,
        trim: Self::NO_TRIM,
    };

    /// The requested byte is an sRGB code value.
    pub const SRGB: Self = Self {
        decode: true,
        trim: Self::NO_TRIM,
    };

    /// Attenuate individual channels to move the chain's white point.
    ///
    /// Raw emitters are green-heavy, so `#ffffff` reads green without a trim.
    /// The values belong to a specific emitter and must be measured on one,
    /// never guessed: a wrong trim tints every colour the board can show.
    pub const fn with_trim(self, trim: Rgb8) -> Self {
        Self {
            decode: self.decode,
            trim,
        }
    }

    /// The duty the chain should receive for `pixel` under `ceiling`.
    pub const fn apply(self, pixel: Rgb8, ceiling: u8) -> Rgb8 {
        Rgb8::new(
            self.channel(pixel.r, self.trim.r, ceiling),
            self.channel(pixel.g, self.trim.g, ceiling),
            self.channel(pixel.b, self.trim.b, ceiling),
        )
    }

    const fn channel(self, value: u8, trim: u8, ceiling: u8) -> u8 {
        let light = if self.decode {
            SRGB_DECODE[value as usize]
        } else {
            value
        };
        limit_channel(limit_channel(light, trim), ceiling)
    }
}

pub const fn limit_channel(channel: u8, ceiling: u8) -> u8 {
    ((channel as u16 * ceiling as u16 + u8::MAX as u16 / 2) / u8::MAX as u16) as u8
}

pub fn frame_visible(frame: &[Rgb8], profile: ColorProfile, ceiling: u8) -> bool {
    frame
        .iter()
        .any(|pixel| profile.apply(*pixel, ceiling) != Rgb8::BLACK)
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
