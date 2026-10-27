//! Accolade icons, drawn as terminal images where the terminal supports a graphics protocol.
//!
//! The PNGs are the game's own icons (white glyphs, extracted from the 13.08 game files). They are
//! tinted with the game's cool medal color. Without image support the two-letter text placeholders
//! drawn under each icon stay visible instead.

use crate::goldstars::Accolade;
use image::{DynamicImage, Rgba, RgbaImage};
use ratatui::Frame;
use ratatui::layout::{Rect, Size};
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};
use std::collections::HashMap;

const TINT: [u8; 3] = [0xD9, 0xF3, 0xFB];

fn png(kind: Accolade) -> &'static [u8] {
    match kind {
        Accolade::Mvp => include_bytes!("../assets/accolades/mvp.png"),
        Accolade::Masterful => include_bytes!("../assets/accolades/distinction.png"),
        Accolade::TopFrag => include_bytes!("../assets/accolades/top-frag.png"),
        Accolade::Kills => include_bytes!("../assets/accolades/kills.png"),
        Accolade::FirstBlood => include_bytes!("../assets/accolades/first-blood.png"),
        Accolade::Damage => include_bytes!("../assets/accolades/damage.png"),
        Accolade::Headshots => include_bytes!("../assets/accolades/headshot-percentage.png"),
        Accolade::Assists => include_bytes!("../assets/accolades/assists.png"),
        Accolade::Plants => include_bytes!("../assets/accolades/plants.png"),
        Accolade::Aces => include_bytes!("../assets/accolades/aces.png"),
        Accolade::Clutches => include_bytes!("../assets/accolades/clutches.png"),
        Accolade::Trades => include_bytes!("../assets/accolades/trades.png"),
    }
}

fn tinted(kind: Accolade) -> Option<DynamicImage> {
    let src = image::load_from_memory(png(kind)).ok()?.to_rgba8();
    let out = RgbaImage::from_fn(src.width(), src.height(), |x, y| {
        let a = src.get_pixel(x, y)[3];
        Rgba([TINT[0], TINT[1], TINT[2], a])
    });
    Some(DynamicImage::ImageRgba8(out))
}

#[derive(Default)]
pub struct Icons {
    protocols: HashMap<Accolade, Protocol>,
}

impl Icons {
    /// Encodes every icon for a 2x1-cell slot. Half-block "images" that small are unreadable, so
    /// terminals without a real graphics protocol get no images and keep the text placeholders.
    pub fn new(picker: Option<&Picker>) -> Self {
        let Some(picker) = picker.filter(|p| p.protocol_type() != ProtocolType::Halfblocks) else {
            return Self::default();
        };
        let protocols = Accolade::ALL
            .iter()
            .filter_map(|&k| Some((k, picker.new_protocol(tinted(k)?, Size::new(2, 1), Resize::Fit(None)).ok()?)))
            .collect();
        Self { protocols }
    }

    pub fn draw(&self, f: &mut Frame, kind: Accolade, area: Rect) {
        if let Some(p) = self.protocols.get(&kind) {
            f.render_widget(Image::new(p), area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_decodes_and_keeps_alpha() {
        for k in Accolade::ALL {
            let img = tinted(k).unwrap_or_else(|| panic!("{k:?} failed to decode")).to_rgba8();
            assert_eq!((img.width(), img.height()), (256, 256), "{k:?}");
            let opaque = img.pixels().filter(|p| p[3] > 0).count();
            assert!(opaque > 0 && opaque < 256 * 256, "{k:?} should be a glyph on transparency");
            assert!(img.pixels().all(|p| p[0] == TINT[0] && p[1] == TINT[1] && p[2] == TINT[2]));
        }
    }
}
