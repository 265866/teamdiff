//! Renders a screen into ratatui's `TestBackend`, for the tests.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use teamdiff_core::model::Model;

use crate::draw::{self, UiState};
use crate::icons::Icons;

/// The screen for `m` and `ui` on a `w` by `h` terminal without images, one string per line with
/// its trailing spaces cut.
pub(crate) fn render(m: &Model, ui: &UiState, w: u16, h: u16) -> Vec<String> {
    let buf = render_buffer(m, ui, &Icons::default(), w, h);
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

/// The cells of the screen for `m` and `ui` on a `w` by `h` terminal, drawn with `icons`.
pub(crate) fn render_buffer(m: &Model, ui: &UiState, icons: &Icons, w: u16, h: u16) -> Buffer {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| draw::draw(f, m, ui, icons)).unwrap();
    t.backend().buffer().clone()
}
