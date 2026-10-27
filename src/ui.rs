//! Terminal rendering. Every screen is a fixed-width text grid.

use crate::model::{Model, Phase};
use crate::riot::NetStats;
use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use unicode_width::UnicodeWidthStr;

/// Every screen is this many columns wide; narrower terminals get the too-small screen.
pub const WIDTH: u16 = 112;

// ---------- palette ----------
const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}
fn fg(c: Color) -> Style {
    Style::new().fg(c)
}
fn text() -> Style {
    fg(rgb(0xc9, 0xd1, 0xd9))
}
fn dim() -> Style {
    fg(rgb(0x6b, 0x76, 0x84))
}
fn border() -> Style {
    fg(rgb(0x3b, 0x45, 0x52))
}
fn bold() -> Style {
    fg(rgb(0xf0, 0xf3, 0xf6)).add_modifier(Modifier::BOLD)
}
fn you() -> Style {
    fg(rgb(0xfb, 0xbf, 0x24)).add_modifier(Modifier::BOLD)
}
fn good() -> Style {
    fg(rgb(0x4a, 0xde, 0x80))
}
fn bad() -> Style {
    fg(rgb(0xf8, 0x71, 0x71))
}
fn warn() -> Style {
    fg(rgb(0xfb, 0xbf, 0x24))
}
const PARTY: [Color; 4] = [rgb(0x60, 0xa5, 0xfa), rgb(0xe8, 0x79, 0xf9), rgb(0xfb, 0x92, 0x3c), rgb(0xa3, 0xe6, 0x35)];

// ---------- text grid ----------
type Seg = Span<'static>;

fn s(t: impl Into<String>, style: Style) -> Seg {
    Span::styled(t.into(), style)
}

#[derive(Default, Clone)]
struct Row {
    segs: Vec<Seg>,
}

impl From<Vec<Seg>> for Row {
    fn from(segs: Vec<Seg>) -> Self {
        Row { segs }
    }
}

fn blank() -> Row {
    Row::default()
}

fn keys(pairs: &[(&str, &str)]) -> Vec<Seg> {
    pairs
        .iter()
        .enumerate()
        .flat_map(|(i, (k, d))| [s(format!("{}{k}", if i > 0 { "  " } else { "" }), bold()), s(format!(" {d}"), dim())])
        .collect()
}

const LOADING: &str = "···";
const ERROR_SHOWN_FOR: std::time::Duration = std::time::Duration::from_secs(15);

/// A screen ready to draw.
struct Screen {
    title: Vec<Seg>,
    right: Vec<Seg>,
    rows: Vec<Row>,
    footer: Vec<Seg>,
}

// ---------- shared helpers ----------
fn queue_name(q: &str) -> String {
    match q {
        "competitive" => "Competitive".into(),
        "unrated" => "Unrated".into(),
        "swiftplay" => "Swiftplay".into(),
        "spikerush" => "Spike Rush".into(),
        "deathmatch" => "Deathmatch".into(),
        "hurm" => "Team Deathmatch".into(),
        "ggteam" => "Escalation".into(),
        "premier" => "Premier".into(),
        "" => "Custom".into(),
        other => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        }
    }
}

/// Each player's party marker, one color per party.
fn party_marks(parties: &HashMap<String, String>) -> HashMap<String, Seg> {
    let mut ids: Vec<&String> = parties.values().collect();
    ids.sort();
    ids.dedup();
    let color: HashMap<&String, Color> = ids.into_iter().enumerate().map(|(i, id)| (id, PARTY[i % PARTY.len()])).collect();
    parties.iter().map(|(p, id)| (p.clone(), s("■", fg(color[id])))).collect()
}

fn name_segs(m: &Model, puuid: &str) -> Vec<Seg> {
    let name = m.players.get(puuid).and_then(|d| d.name.clone());
    match (name, puuid == m.me) {
        (Some((n, _)), true) => vec![s(n, you())],
        (None, true) => vec![s("you", you())],
        (Some((n, t)), false) => vec![s(n, text()), s(format!("#{t}"), dim())],
        (None, false) => vec![s(LOADING, dim())],
    }
}

// ---------- menus / waiting ----------
fn waiting_screen(reason: &str) -> Screen {
    Screen {
        title: vec![s("teamdiff", bold())],
        right: vec![s("○ offline", dim())],
        rows: vec![blank(), vec![s("   Waiting for VALORANT…", bold())].into(), vec![s(format!("   {reason}"), dim())].into(), blank()],
        footer: keys(&[("q", "quit")]),
    }
}

fn parse_queue_time(t: &str) -> Option<chrono::DateTime<Utc>> {
    chrono::NaiveDateTime::parse_from_str(t, "%Y.%m.%d-%H.%M.%S").ok().map(|n| n.and_utc())
}

fn menus_screen(m: &Model) -> Screen {
    let now = Utc::now();
    let mut rows = vec![blank()];
    let mut line = vec![s("   ", Style::new())];
    line.extend(name_segs(m, &m.me));
    if let Some(l) = m.my_presence().and_then(|p| p.account_level) {
        line.push(s(format!("   level {l}"), dim()));
    }
    rows.push(line.into());
    rows.push(blank());
    let party = party_marks(&m.parties());
    let mut mates: Vec<&String> = party.keys().filter(|p| **p != m.me).collect();
    // A HashMap's order changes from one frame to the next.
    mates.sort();
    for (i, mate) in mates.iter().enumerate() {
        let mut r = vec![s(if i == 0 { "   Party      " } else { "              " }, dim()), party[*mate].clone(), s(" ", Style::new())];
        r.extend(name_segs(m, mate));
        rows.push(r.into());
    }
    let p = m.my_presence();
    let queue = p.and_then(|p| p.queue_id.clone()).map(|q| queue_name(&q)).unwrap_or_else(|| "—".into());
    let status = match p.and_then(|p| p.party_state.clone()).as_deref() {
        Some("MATCHMAKING") => {
            let waited = p.and_then(|p| p.queue_entry_time.as_deref()).and_then(parse_queue_time).map(|t| (now - t).num_seconds().max(0));
            match waited {
                Some(w) => format!("In queue · {queue} · {}:{:02}", w / 60, w % 60),
                None => format!("In queue · {queue}"),
            }
        }
        _ => format!("In lobby · {queue}"),
    };
    rows.push(vec![s("   Status     ", dim()), s(status, text())].into());
    rows.push(blank());
    Screen {
        title: vec![s("teamdiff", bold())],
        right: vec![s("● ", good()), s(m.region.clone(), dim())],
        rows,
        footer: keys(&[("q", "quit")]),
    }
}

// ---------- frame ----------
fn net_status(net: Option<&NetStats>) -> Vec<Seg> {
    let Some(net) = net else { return Vec::new() };
    let (started, finished) = (net.started.load(Ordering::Relaxed), net.finished.load(Ordering::Relaxed));
    if started == finished {
        return Vec::new();
    }
    vec![s("◐ ", warn()), s(format!("loading {finished}/{started}"), warn())]
}

fn screen(m: &Model) -> Screen {
    match &m.phase {
        Phase::Waiting(reason) => waiting_screen(reason),
        Phase::Menus => menus_screen(m),
    }
}

fn too_small(f: &mut Frame, need_w: u16, need_h: u16) {
    let area = f.area();
    let msg = if area.width < need_w {
        format!("Widen to {need_w} columns (now {})", area.width)
    } else {
        format!("Make it taller: {need_h} rows needed (now {})", area.height)
    };
    let w = (msg.width() as u16 + 6).max(24).min(area.width);
    let h = 6.min(area.height);
    let rect = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    let p =
        Paragraph::new(vec![Line::default(), Line::from(s("Terminal too small", bold())).centered(), Line::from(s(msg, dim())).centered()])
            .block(Block::bordered().border_type(BorderType::Plain).border_style(border()));
    f.render_widget(p, rect);
}

pub fn draw(f: &mut Frame, m: &Model) {
    let sc = screen(m);
    let status = match &m.last_error {
        Some((e, at)) if at.elapsed() < ERROR_SHOWN_FOR => vec![s("⚠ ", bad()), s(e.chars().take(60).collect::<String>(), bad())],
        _ => net_status(m.net.as_deref()),
    };
    let need_h = sc.rows.len() as u16 + 2;
    let area = f.area();
    if area.width < WIDTH || area.height < need_h {
        too_small(f, WIDTH, need_h);
        return;
    }
    let rect = Rect { x: area.x, y: area.y, width: WIDTH, height: need_h };
    let mut block = Block::bordered()
        .border_style(border())
        .padding(Padding::horizontal(1))
        .title(Line::from([vec![s("─ ", border())], sc.title, vec![s(" ", border())]].concat()));
    if !sc.right.is_empty() {
        block = block.title(Line::from([vec![s(" ", border())], sc.right, vec![s(" ─", border())]].concat()).right_aligned());
    }
    block = block.title_bottom(Line::from([vec![s("─ ", border())], sc.footer, vec![s(" ", border())]].concat()));
    if !status.is_empty() {
        block = block.title_bottom(Line::from([vec![s(" ", border())], status, vec![s(" ─", border())]].concat()).right_aligned());
    }
    let lines: Vec<Line> = sc.rows.iter().map(|r| Line::from(r.segs.clone())).collect();
    f.render_widget(Paragraph::new(lines).style(text()).block(block), rect);
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::Instant;

    pub fn render(m: &Model, w: u16, h: u16) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, m)).unwrap();
        let buf = t.backend().buffer();
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect()
    }

    #[test]
    fn request_errors_show_in_the_footer() {
        let mut m = Model::new();
        let err = "500 Internal Server Error from /name-service/v2/players, with a reason long enough to be cut off".to_string();
        m.last_error = Some((err.clone(), Instant::now()));
        let lines = render(&m, WIDTH, 10);
        let footer = lines.iter().find(|l| l.starts_with('└')).unwrap();
        assert!(footer.contains("⚠ 500 Internal Server Error from /name-service/v2/players"), "the status survives truncation: {footer}");
        m.last_error = Some((err, Instant::now() - ERROR_SHOWN_FOR));
        let lines = render(&m, WIDTH, 10);
        assert!(!lines.iter().any(|l| l.contains('⚠')), "old errors fade out");
    }

    #[test]
    fn too_small_screen_reports_width() {
        let m = Model::new();
        let lines = render(&m, 80, 20);
        assert!(lines.iter().any(|l| l.contains("Terminal too small")));
        assert!(lines.iter().any(|l| l.contains("Widen to 112 columns (now 80)")));
    }
}
