//! The screenshots in `docs/screenshots/`: each demo screen drawn as `teamdiff --demo` draws it
//! and written as an SVG of a terminal window, and the check that every capture there shows only
//! the demo's made-up data.
//!
//! `TEAMDIFF_WRITE_SCREENSHOTS=1 cargo test -p teamdiff_ui screenshots` rewrites the files; without
//! the variable the test only checks them.

use std::collections::{BTreeSet, HashSet};
use std::fmt::{self, Write as _};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::{env, fs};

use image::ImageFormat;
use image::imageops::{self, FilterType};
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::style::{Color, Modifier};
use ratatui_image::picker::{Picker, ProtocolType};
use teamdiff_core::demo;
use teamdiff_core::goldstars::Accolade;
use unicode_width::UnicodeWidthStr;

use crate::draw::{UiState, View, WIDTH};
use crate::icons::{self, Icons};
use crate::{cast, rendering};

/// Tall enough for every screen; the capture keeps only the rows the screen draws.
const HEIGHT: u16 = 60;
/// A cell's width and height in the image, in pixels.
const CW: f64 = 8.4;
const CH: f64 = 18.0;
const FONT_SIZE: f64 = 14.0;
/// From a cell's top to the text's baseline.
const BASELINE: f64 = 13.5;
/// Space around the screen, and the title bar's height.
const PAD: f64 = 12.0;
const BAR: f64 = 32.0;
/// Windows Terminal's default background and text, for cells the screen leaves unstyled.
const TERMINAL_BG: &str = "#0c0c0c";
const TERMINAL_FG: &str = "#cccccc";
const FONTS: &str = "'Cascadia Mono', 'Cascadia Code', Consolas, 'SF Mono', Menlo, \
                     'DejaVu Sans Mono', 'Liberation Mono', monospace";
/// The side of an accolade icon as embedded, in pixels: sharp at twice the slot's size.
const ICON_PIXELS: u32 = 40;

/// One screenshot: the file it's written to, the demo screen and the UI state it shows.
struct Capture {
    file: &'static str,
    screen: &'static str,
    view: Option<&'static str>,
    legend: bool,
}

const fn shot(file: &'static str, screen: &'static str) -> Capture {
    Capture {
        file,
        screen,
        view: None,
        legend: false,
    }
}

/// Every screenshot, in the gallery's order.
const CAPTURES: [Capture; 10] = [
    shot("waiting", "waiting"),
    shot("menus", "menus"),
    shot("agent-select", "agent-select"),
    shot("loading", "loading"),
    shot("in-game", "in-game"),
    Capture {
        view: Some("raze"),
        ..shot("player-details", "in-game")
    },
    Capture {
        view: Some("me"),
        ..shot("profile", "in-game")
    },
    shot("victory", "victory"),
    Capture {
        legend: true,
        ..shot("legend", "victory")
    },
    shot("defeat", "defeat"),
];

fn screenshots_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots")
}

/// The SVG of `c`: the demo's model for its screen, drawn by the same code as the app, with
/// the post-game banner as it stands once its letters have appeared rather than mid-animation.
fn capture(c: &Capture) -> String {
    let mut model = demo::model(c.screen);
    if let Some(pg) = model.postgame.as_mut() {
        pg.shown_at = None;
    }
    let ui = UiState {
        view: c.view.map_or(View::Main, |id| View::Detail(id.into())),
        legend: c.legend,
        ..Default::default()
    };
    let text = rendering::render_buffer(&model, &ui, &Icons::default(), WIDTH, HEIGHT);
    let mut picker = Picker::halfblocks();
    picker.set_protocol_type(ProtocolType::Sixel);
    let images = rendering::render_buffer(&model, &ui, &Icons::new(Some(&picker)), WIDTH, HEIGHT);
    svg(&text, &icon_cells(&text, &images)).unwrap()
}

/// An accolade icon the terminal draws, and the cell its image starts in.
struct IconCell {
    at: Position,
    kind: Accolade,
}

/// Where the terminal draws an accolade icon: the cell an image starts in, and the accolade its
/// two-letter placeholder in the text-only drawing names.
fn icon_cells(text: &Buffer, images: &Buffer) -> Vec<IconCell> {
    let mut cells = Vec::new();
    for y in 0..HEIGHT {
        for x in 0..WIDTH.saturating_sub(1) {
            if images[(x, y)].symbol().contains("\u{1b}P") {
                let abbrev = format!("{}{}", text[(x, y)].symbol(), text[(x + 1, y)].symbol());
                let kind = Accolade::ALL
                    .into_iter()
                    .find(|k| k.abbrev() == abbrev)
                    .unwrap_or_else(|| panic!("no accolade abbreviated {abbrev:?} at {x},{y}"));
                cells.push(IconCell {
                    at: Position::new(x, y),
                    kind,
                });
            }
        }
    }
    cells
}

fn hex(c: Color, default: &str) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset => default.to_owned(),
        other => panic!("the screen uses only RGB colors, not {other:?}"),
    }
}

fn px(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0")
        .map_or_else(|| s.clone(), str::to_owned)
}

fn left(x: u16) -> f64 {
    PAD + f64::from(x) * CW
}

fn top(y: u16) -> f64 {
    BAR + PAD + f64::from(y) * CH
}

/// The cell's line segments, for box-drawing characters: up, down, left, right.
fn box_lines(symbol: &str) -> Option<[bool; 4]> {
    Some(match symbol {
        "─" => [false, false, true, true],
        "│" => [true, true, false, false],
        "┌" => [false, true, false, true],
        "┐" => [false, true, true, false],
        "└" => [true, false, false, true],
        "┘" => [true, false, true, false],
        "├" => [true, true, false, true],
        "┤" => [true, true, true, false],
        "┬" => [false, true, true, true],
        "┴" => [true, false, true, true],
        "┼" => [true, true, true, true],
        _ => return None,
    })
}

/// The part of the cell a block element fills: left, top, width and height, as fractions.
fn block_fill(symbol: &str) -> Option<[f64; 4]> {
    let lower = |eighths: f64| [0.0, 1.0 - eighths / 8.0, 1.0, eighths / 8.0];
    Some(match symbol {
        "█" => lower(8.0),
        "▇" => lower(7.0),
        "▆" => lower(6.0),
        "▅" => lower(5.0),
        "▄" => lower(4.0),
        "▃" => lower(3.0),
        "▂" => lower(2.0),
        "▁" => lower(1.0),
        "▀" => [0.0, 0.0, 1.0, 0.5],
        "▌" => [0.0, 0.0, 0.5, 1.0],
        "▐" => [0.5, 0.0, 0.5, 1.0],
        _ => return None,
    })
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = usize::try_from((n >> (18 - 6 * i)) & 63).unwrap();
                out.push(char::from(BASE64[index]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The accolade's icon, tinted as the terminal shows it, as a small PNG.
fn icon_png(kind: Accolade) -> String {
    let icon = icons::tinted(kind).unwrap();
    let small = imageops::resize(&icon, ICON_PIXELS, ICON_PIXELS, FilterType::Lanczos3);
    let mut png = Cursor::new(Vec::new());
    small.write_to(&mut png, ImageFormat::Png).unwrap();
    base64(&png.into_inner())
}

/// The rows the screen draws: up to the last one with anything in it.
fn drawn_rows(buf: &Buffer) -> u16 {
    (0..HEIGHT)
        .rev()
        .find(|&y| (0..WIDTH).any(|x| !buf[(x, y)].symbol().trim().is_empty()))
        .map_or(0, |y| y + 1)
}

/// A run of text in one style on one row, ending before the cell `end`.
struct Run {
    end: u16,
    fg: String,
    bold: bool,
    italic: bool,
    text: String,
    xs: Vec<f64>,
}

/// The terminal window: a title bar, then every cell of `buf`. Box-drawing lines and block
/// elements are drawn as shapes, so borders join and bars fill whatever font the viewer has.
fn svg(buf: &Buffer, icons: &[IconCell]) -> Result<String, fmt::Error> {
    let rows = drawn_rows(buf);
    let width = PAD * 2.0 + f64::from(WIDTH) * CW;
    let height = BAR + PAD * 2.0 + f64::from(rows) * CH;
    let mut out = String::new();
    let (w, h) = (px(width), px(height));
    writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" \
         viewBox=\"0 0 {w} {h}\">"
    )?;
    plain_text(&mut out, buf, rows)?;
    let kinds: BTreeSet<Accolade> = icons.iter().map(|icon| icon.kind).collect();
    if !kinds.is_empty() {
        out.push_str("<defs>\n");
        let side = px(CW * 2.0);
        for k in &kinds {
            writeln!(
                out,
                "<image id=\"{}\" width=\"{side}\" height=\"{side}\" href=\"data:image/png;base64,{}\"/>",
                k.abbrev(),
                icon_png(*k)
            )?;
        }
        out.push_str("</defs>\n");
    }
    writeln!(
        out,
        "<rect x=\"0.5\" y=\"0.5\" width=\"{}\" height=\"{}\" rx=\"8\" fill=\"{TERMINAL_BG}\" \
         stroke=\"#333333\"/>",
        px(width - 1.0),
        px(height - 1.0)
    )?;
    title_bar(&mut out, width)?;
    backgrounds(&mut out, buf, rows)?;
    shapes(&mut out, buf, rows)?;
    let mut covered = HashSet::new();
    for &IconCell { at, kind: k } in icons {
        let Position { x, y } = at;
        covered.extend([at, Position::new(x + 1, y)]);
        writeln!(
            out,
            "<use href=\"#{}\" x=\"{}\" y=\"{}\"/>",
            k.abbrev(),
            px(left(x)),
            px(top(y) + (CH - CW * 2.0) / 2.0)
        )?;
    }
    writeln!(
        out,
        "<g font-family=\"{FONTS}\" font-size=\"{}\">",
        px(FONT_SIZE)
    )?;
    for y in 0..rows {
        for run in runs(buf, y, &covered) {
            text_element(&mut out, &run, y)?;
        }
    }
    out.push_str("</g>\n</svg>\n");
    Ok(out)
}

/// The screen as plain text in `<desc>`, for screen readers, and so `scripts/privacy-check.sh`
/// sees each name with its tag, which the drawing colors apart.
fn plain_text(out: &mut String, buf: &Buffer, rows: u16) -> fmt::Result {
    out.push_str("<desc>\n");
    for y in 0..rows {
        let line: String = (0..WIDTH).map(|x| buf[(x, y)].symbol()).collect();
        writeln!(out, "{}", escape(line.trim_end()))?;
    }
    out.push_str("</desc>\n");
    Ok(())
}

fn title_bar(out: &mut String, width: f64) -> fmt::Result {
    writeln!(
        out,
        "<path d=\"M0.5 {BAR}V8.5a8 8 0 0 1 8-8H{}a8 8 0 0 1 8 8V{BAR}Z\" fill=\"#202020\"/>",
        px(width - 8.5)
    )?;
    writeln!(
        out,
        "<text x=\"{PAD}\" y=\"21\" font-family=\"'Segoe UI', system-ui, sans-serif\" \
         font-size=\"12\" fill=\"{TERMINAL_FG}\">teamdiff</text>"
    )?;
    let controls = width - 30.0;
    writeln!(
        out,
        "<path d=\"M{} 16.5h10M{} 11.5h9v9h-9zM{} 11l9 9M{} 11l-9 9\" stroke=\"#bbbbbb\" \
         fill=\"none\"/>",
        px(controls - 92.0),
        px(controls - 46.5),
        px(controls),
        px(controls + 9.0)
    )?;
    Ok(())
}

fn backgrounds(out: &mut String, buf: &Buffer, rows: u16) -> fmt::Result {
    for y in 0..rows {
        let mut x = 0;
        while x < WIDTH {
            let bg = buf[(x, y)].bg;
            let start = x;
            while x < WIDTH && buf[(x, y)].bg == bg {
                x += 1;
            }
            if bg != Color::Reset {
                writeln!(
                    out,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
                    px(left(start)),
                    px(top(y)),
                    px(f64::from(x - start) * CW),
                    px(CH),
                    hex(bg, TERMINAL_BG)
                )?;
            }
        }
    }
    Ok(())
}

fn shapes(out: &mut String, buf: &Buffer, rows: u16) -> fmt::Result {
    let mut lines: Vec<(String, String)> = Vec::new();
    for y in 0..rows {
        for x in 0..WIDTH {
            let cell = &buf[(x, y)];
            let fg = hex(cell.fg, TERMINAL_FG);
            let path = if let Some([up, down, l, r]) = box_lines(cell.symbol()) {
                let (cx, cy) = (left(x) + CW / 2.0, top(y) + CH / 2.0);
                let mut d = String::new();
                if l || r {
                    let from = if l { left(x) } else { cx };
                    let to = if r { left(x) + CW } else { cx };
                    write!(d, "M{} {}H{}", px(from), px(cy.floor() + 0.5), px(to))?;
                }
                if up || down {
                    let from = if up { top(y) } else { cy };
                    let to = if down { top(y) + CH } else { cy };
                    write!(d, "M{} {}V{}", px(cx.floor() + 0.5), px(from), px(to))?;
                }
                d
            } else if let Some([fx, fy, fw, fh]) = block_fill(cell.symbol()) {
                format!(
                    "M{} {}h{}v{}h-{}z",
                    px(left(x) + fx * CW),
                    px(top(y) + fy * CH),
                    px(fw * CW),
                    px(fh * CH),
                    px(fw * CW)
                )
            } else {
                continue;
            };
            match lines.iter_mut().find(|(c, _)| *c == fg) {
                Some((_, d)) => d.push_str(&path),
                None => lines.push((fg, path)),
            }
        }
    }
    for (fg, d) in lines {
        writeln!(
            out,
            "<path d=\"{d}\" stroke=\"{fg}\" fill=\"{fg}\" stroke-width=\"1\"/>"
        )?;
    }
    Ok(())
}

/// The row's words, split at spaces and where the style changes, without the cells drawn as
/// shapes or covered by an icon. Spaces stay out of the text, since a viewer may collapse them.
fn runs(buf: &Buffer, y: u16, covered: &HashSet<Position>) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for x in 0..WIDTH {
        let cell = &buf[(x, y)];
        let symbol = cell.symbol();
        if symbol.is_empty()
            || symbol.trim().is_empty()
            || covered.contains(&Position::new(x, y))
            || box_lines(symbol).is_some()
            || block_fill(symbol).is_some()
        {
            continue;
        }
        let fg = hex(cell.fg, TERMINAL_FG);
        let bold = cell.modifier.contains(Modifier::BOLD);
        let italic = cell.modifier.contains(Modifier::ITALIC);
        let end = x + cast::low_u16(symbol.width().max(1));
        let joins = runs
            .last()
            .is_some_and(|r| r.fg == fg && r.bold == bold && r.italic == italic && r.end == x);
        if joins {
            let r = runs.last_mut().unwrap();
            r.text.push_str(symbol);
            r.xs.push(left(x));
            r.end = end;
        } else {
            runs.push(Run {
                end,
                fg,
                bold,
                italic,
                text: symbol.to_owned(),
                xs: vec![left(x)],
            });
        }
    }
    runs
}

fn text_element(out: &mut String, run: &Run, y: u16) -> fmt::Result {
    let xs: Vec<String> = run.xs.iter().map(|&x| px(x)).collect();
    let mut attrs = format!("fill=\"{}\"", run.fg);
    if run.bold {
        attrs.push_str(" font-weight=\"bold\"");
    }
    if run.italic {
        attrs.push_str(" font-style=\"italic\"");
    }
    writeln!(
        out,
        "<text x=\"{}\" y=\"{}\" {attrs}>{}</text>",
        xs.join(" "),
        px(top(y) + BASELINE),
        escape(&run.text)
    )?;
    Ok(())
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let start = tag.find(&format!(" {name}=\""))? + name.len() + 3;
    tag[start..].split_once('"').map(|(value, _)| value)
}

/// The screen's text rows in an SVG this module wrote: every character of every text element
/// put back in its cell, so a name and its differently colored tag read as one word again.
/// Characters past an element's last position follow one another, as SVG draws them. A capture
/// never overlaps text; in an edited file the first text keeps its cells, so an inserted value
/// stays whole for the check.
fn svg_text(svg: &str) -> Vec<String> {
    let mut rows: Vec<(String, Vec<char>)> = Vec::new();
    for element in svg.split("<text ").skip(1) {
        let Some((tag, rest)) = element.split_once('>') else {
            continue;
        };
        let tag = format!(" {tag}");
        let (Some(xs), Some(y), Some((content, _))) = (
            attribute(&tag, "x"),
            attribute(&tag, "y"),
            rest.split_once("</text>"),
        ) else {
            continue;
        };
        let content = content
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        let i = rows.iter().position(|(ry, _)| ry == y).unwrap_or_else(|| {
            rows.push((y.to_owned(), Vec::new()));
            rows.len() - 1
        });
        let row = &mut rows[i].1;
        let mut positions = xs.split(' ').filter_map(|x| x.parse::<f64>().ok());
        let mut x = PAD - CW;
        for c in content.chars() {
            x = positions.next().unwrap_or(x + CW);
            let mut cell = 0usize;
            while left(cast::low_u16(cell)) + CW / 2.0 < x {
                cell += 1;
            }
            if row.len() <= cell {
                row.resize(cell + 1, ' ');
            }
            if row[cell] == ' ' {
                row[cell] = c;
            }
        }
    }
    rows.into_iter()
        .map(|(_, row)| row.into_iter().collect())
        .collect()
}

/// Every number as 0, so a timer, a date or the version in the footer doesn't count as new.
fn masked_words(texts: &[String]) -> BTreeSet<String> {
    texts
        .iter()
        .flat_map(|t| t.split_whitespace())
        .map(|w| {
            let mut masked = String::new();
            for c in w.chars() {
                if !c.is_ascii_digit() {
                    masked.push(c);
                } else if !masked.ends_with('0') {
                    masked.push('0');
                }
            }
            masked
        })
        .collect()
}

/// What a capture shows that the demo doesn't: a Riot ID that isn't a demo player's, a UUID, or
/// a word that the demo's drawing of the same screen doesn't have.
fn findings(file: &str, committed: &str, fresh: &str) -> Vec<String> {
    let demo_ids: BTreeSet<String> = demo::SCREENS
        .iter()
        .flat_map(|s| demo::model(s).players.into_values())
        .filter_map(|p| p.name)
        .map(|id| format!("{}#{}", id.name, id.tag))
        .collect();
    let texts = svg_text(committed);
    let mut found = Vec::new();
    let words: Vec<&str> = texts.iter().flat_map(|t| t.split_whitespace()).collect();
    for word in words {
        if word.contains('#') && !demo_ids.contains(word) {
            found.push(format!("{file}: Riot ID {word:?} isn't one of the demo's"));
        }
        let hex_digits = word.chars().filter(char::is_ascii_hexdigit).count();
        if word.matches('-').count() == 4 && hex_digits >= 32 {
            found.push(format!("{file}: {word:?} looks like a UUID"));
        }
    }
    let demo_words = masked_words(&svg_text(fresh));
    for word in masked_words(&texts).difference(&demo_words) {
        found.push(format!(
            "{file}: {word:?} isn't on the demo's screen; rewrite the screenshots, and if it \
             stays, it isn't demo data"
        ));
    }
    found
}

/// Every `screenshots/<name>.svg` the README and the docs show.
fn referenced(root: &Path) -> BTreeSet<String> {
    let mut pages = vec![root.join("README.md")];
    if let Ok(entries) = fs::read_dir(root.join("docs")) {
        pages.extend(entries.filter_map(Result::ok).map(|e| e.path()));
    }
    let mut names = BTreeSet::new();
    pages.retain(|p| p.extension().is_some_and(|e| e == "md"));
    for page in &pages {
        let Ok(text) = fs::read_to_string(page) else {
            continue;
        };
        for part in text.split("screenshots/").skip(1) {
            if let Some((name, _)) = part.split_once(".svg")
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            {
                names.insert(name.to_owned());
            }
        }
    }
    names
}

#[test]
fn screenshots_show_only_demo_data() {
    let dir = screenshots_dir();
    let fresh: Vec<(&str, String)> = CAPTURES.iter().map(|c| (c.file, capture(c))).collect();
    if env::var_os("TEAMDIFF_WRITE_SCREENSHOTS").is_some() {
        fs::create_dir_all(&dir).unwrap();
        for (file, svg) in &fresh {
            fs::write(dir.join(format!("{file}.svg")), svg).unwrap();
        }
    }
    let on_disk: BTreeSet<String> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let expected: BTreeSet<String> = CAPTURES.iter().map(|c| format!("{}.svg", c.file)).collect();
    assert_eq!(
        on_disk, expected,
        "docs/screenshots holds exactly the captures; rewrite them with \
         TEAMDIFF_WRITE_SCREENSHOTS=1 cargo test -p teamdiff_ui screenshots"
    );
    let mut found = Vec::new();
    for (file, svg) in &fresh {
        found.extend(findings(file, svg, svg));
        let committed = fs::read_to_string(dir.join(format!("{file}.svg"))).unwrap();
        found.extend(findings(file, &committed, svg));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let names: BTreeSet<String> = CAPTURES.iter().map(|c| c.file.to_owned()).collect();
    let missing: Vec<String> = referenced(&root).difference(&names).cloned().collect();
    assert!(
        missing.is_empty(),
        "the docs show screenshots no capture writes: {missing:?}"
    );
}

#[test]
fn the_check_refuses_a_real_riot_id_and_a_word_the_demo_lacks() {
    let fresh = capture(&CAPTURES[4]);
    assert_eq!(findings("in-game", &fresh, &fresh), Vec::<String>::new());
    let leaked = fresh.replacen(">Echo<", ">Kilo<", 1);
    assert_ne!(leaked, fresh, "the capture draws a name apart from its tag");
    let found = findings("in-game", &leaked, &fresh).join("\n");
    assert!(found.contains("Riot ID \"Kilo#DEMO\""), "{found}");
    assert!(
        found.contains("\"Kilo#DEMO\" isn't on the demo's screen"),
        "{found}"
    );
    let renamed = fresh.replacen(">Alpha<", ">Zulu<", 1);
    assert_ne!(
        renamed, fresh,
        "the capture draws the player's own name as one text element"
    );
    let found = findings("in-game", &renamed, &fresh).join("\n");
    assert!(
        found.contains("\"Zulu\" isn't on the demo's screen"),
        "{found}"
    );
    let uuid = fresh.replacen(">Alpha<", ">00000000-0000-0000-0000-000000000000<", 1);
    assert!(
        findings("in-game", &uuid, &fresh)
            .join("\n")
            .contains("looks like a UUID")
    );
}

#[test]
fn a_version_or_timer_change_isnt_a_finding() {
    let fresh = capture(&CAPTURES[4]);
    let bumped = fresh.replace(concat!("v", env!("CARGO_PKG_VERSION")), "v9.8.7");
    assert_eq!(findings("in-game", &bumped, &fresh), Vec::<String>::new());
}

#[test]
fn base64_matches_the_standard_alphabet_and_padding() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"M"), "TQ==");
    assert_eq!(base64(b"Ma"), "TWE=");
    assert_eq!(base64(b"Man"), "TWFu");
    assert_eq!(base64(b"Many"), "TWFueQ==");
}
