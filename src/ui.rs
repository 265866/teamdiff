//! Terminal rendering. Every screen is a fixed-width text grid.

use crate::content::Content;
use crate::model::{Load, Lobby, Model, Phase, Pick, Side, Slot};
use crate::riot::NetStats;
use crate::stats::{Record, RecordSource, pooled_kd_hs};
use chrono::Utc;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Instant;
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
fn ally() -> Style {
    fg(rgb(0x4f, 0xd1, 0xc5))
}
fn enemy() -> Style {
    fg(rgb(0xf8, 0x71, 0x71))
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

fn rank_style(name: &str) -> Style {
    let c = match name.split_whitespace().next().unwrap_or("") {
        "Iron" => rgb(0x8b, 0x8b, 0x8b),
        "Bronze" => rgb(0xb0, 0x8d, 0x57),
        "Silver" => rgb(0xc0, 0xc7, 0xcf),
        "Gold" => rgb(0xe5, 0xb9, 0x4e),
        "Platinum" => rgb(0x3f, 0xb8, 0xc4),
        "Diamond" => rgb(0xc0, 0x84, 0xfc),
        "Ascendant" => rgb(0x34, 0xd3, 0x99),
        "Immortal" => rgb(0xf0, 0x5a, 0x6a),
        "Radiant" => rgb(0xff, 0xf3, 0xb0),
        _ => rgb(0x6b, 0x76, 0x84),
    };
    fg(c)
}

fn tier_seg(content: &Content, tier: u32) -> Seg {
    let name = content.tier_name(tier);
    let style = rank_style(&name);
    s(name, style)
}

fn pct_style(p: u32) -> Style {
    if p >= 55 {
        good()
    } else if p <= 45 {
        bad()
    } else {
        text()
    }
}

// ---------- text grid ----------
type Seg = Span<'static>;

fn s(t: impl Into<String>, style: Style) -> Seg {
    Span::styled(t.into(), style)
}

fn width(segs: &[Seg]) -> usize {
    segs.iter().map(|s| s.content.width()).sum()
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

/// Truncates with an ellipsis or pads to exactly `w` columns.
fn fit(segs: Vec<Seg>, w: usize, right: bool) -> Vec<Seg> {
    let n = width(&segs);
    if n > w {
        let mut out = Vec::new();
        let mut left = w.saturating_sub(1);
        for seg in segs {
            if left == 0 {
                break;
            }
            let mut t = String::new();
            for ch in seg.content.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if cw > left {
                    left = 0;
                    break;
                }
                t.push(ch);
                left -= cw;
            }
            out.push(Span::styled(t, seg.style));
        }
        let used = width(&out);
        out.push(s("…", dim()));
        if used + 1 < w {
            out.push(s(" ".repeat(w - used - 1), Style::new()));
        }
        return out;
    }
    let pad = s(" ".repeat(w - n), Style::new());
    if right { std::iter::once(pad).chain(segs).collect() } else { segs.into_iter().chain(std::iter::once(pad)).collect() }
}

struct Col {
    title: String,
    w: usize,
    right: bool,
}

fn col(title: &str, w: usize, right: bool) -> Col {
    Col { title: title.into(), w, right }
}

/// Joins cells into a row, separated by one space.
fn join(cols: &[Col], cells: Vec<Vec<Seg>>) -> Row {
    let mut row = Row::default();
    for (i, (c, cell)) in cols.iter().zip(cells).enumerate() {
        if i > 0 {
            row.segs.push(s(" ", Style::new()));
        }
        row.segs.extend(fit(cell, c.w, c.right));
    }
    row
}

fn header(cols: &[Col]) -> Row {
    join(cols, cols.iter().map(|c| vec![s(c.title.clone(), dim())]).collect())
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

fn name_segs(m: &Model, puuid: &str, incognito: bool, agent: Option<&str>) -> Vec<Seg> {
    // The game hides an incognito player's name from others, never from themselves.
    if incognito && puuid != m.me {
        return match agent {
            Some(a) => vec![s(m.content.agent(a), text().add_modifier(Modifier::ITALIC))],
            None => vec![s("Player", dim().add_modifier(Modifier::ITALIC))],
        };
    }
    let name = m.players.get(puuid).and_then(|d| d.name.clone());
    match (name, puuid == m.me) {
        (Some((n, _)), true) => vec![s(n, you())],
        (None, true) => vec![s("you", you())],
        (Some((n, t)), false) => vec![s(n, text()), s(format!("#{t}"), dim())],
        (None, false) => vec![s(LOADING, dim())],
    }
}

fn rank_segs(content: &Content, tier: u32, rr: Option<u32>) -> Vec<Seg> {
    if tier == 0 {
        return vec![s("Unranked", dim())];
    }
    let mut segs = vec![tier_seg(content, tier)];
    segs.extend(rr.map(|rr| s(format!(" {rr}rr"), dim())));
    segs
}

fn record_segs(r: Record, dim_all: bool) -> Vec<Seg> {
    let Some(p) = r.pct() else { return vec![s("  —", dim())] };
    vec![s(format!("{p:>3}%"), if dim_all { dim() } else { pct_style(p) }), s(format!(" {}-{}", r.wins, r.losses), dim())]
}

/// The current act's id and the one before it.
fn act_ids(m: &Model) -> (Option<String>, Option<String>) {
    let current = m.content.current_act(Utc::now());
    let previous = current.and_then(|a| m.content.previous_act(&a.id)).map(|a| a.id.clone());
    (current.map(|a| a.id.clone()), previous)
}

// ---------- lobby ----------
fn lobby_cols(map: &str) -> Vec<Col> {
    let upper = map.to_uppercase();
    let map_title = if upper.len() + 8 <= 14 { format!("WIN% ON {upper}") } else { format!("{upper} WIN%") };
    vec![
        col("", 1, false),
        col("AGENT", 11, false),
        col("PLAYER", 18, false),
        col("LVL", 4, true),
        col("RANK", 16, false),
        col("PEAK", 11, false),
        col("WIN% THIS ACT", 15, false),
        col(&map_title, 14, false),
        col("K/D", 4, true),
        col("HS%", 4, true),
    ]
}

fn last5_header(cols: &[Col]) -> Row {
    let start: usize = cols[..8].iter().enumerate().map(|(i, c)| c.w + usize::from(i > 0)).sum::<usize>() + 1;
    vec![s(" ".repeat(start), Style::new()), s(" LAST 5  ", dim())].into()
}

fn lobby_row(m: &Model, lobby: &Lobby, slot: &Slot, cols: &[Col], pregame: bool, party: &HashMap<String, Seg>) -> Row {
    let content = &m.content;
    let party_cell = match party.get(&slot.puuid) {
        Some(mark) => vec![mark.clone()],
        None => vec![s(" ", Style::new())],
    };
    let agent_name = slot.agent_id.as_deref().map(|a| content.agent(a));
    let agent = match (pregame, slot.pick, &agent_name) {
        (false, _, Some(a)) => vec![s(a.clone(), bold())],
        (true, Pick::Locked, Some(a)) => vec![s("◆ ", dim()), s(a.clone(), bold())],
        (true, Pick::Hover, Some(a)) => vec![s("◇ ", dim()), s(a.clone(), dim().add_modifier(Modifier::ITALIC))],
        _ => vec![s("  —", dim())],
    };
    let shown_agent = if pregame && slot.pick == Pick::None { None } else { slot.agent_id.as_deref() };
    let name = name_segs(m, &slot.puuid, slot.incognito, shown_agent);
    let lvl = match (slot.hide_level, slot.level) {
        (false, Some(l)) => vec![s(l.to_string(), text())],
        _ => vec![s("—", dim())],
    };
    let data = m.players.get(&slot.puuid);
    let loading = || vec![s(LOADING, dim())];
    let dash = || vec![s("—", dim())];
    let no_record = || vec![s("  —", dim())];
    // Incognito players are never looked up, so they have no stats.
    let hidden = slot.incognito && slot.puuid != m.me;
    let (current, previous) = act_ids(m);
    let (rank, peak, act) = match data.map(|d| &d.profile) {
        _ if hidden => (vec![s("incognito", dim())], dash(), no_record()),
        None | Some(Load::Loading) => (loading(), loading(), loading()),
        Some(Load::Missing) => (vec![s("not found", dim())], dash(), no_record()),
        Some(Load::Private) => (vec![s("private", dim())], dash(), no_record()),
        Some(Load::Ready(p)) => {
            let peak = if p.peak_tier == 0 { dash() } else { vec![tier_seg(content, p.peak_tier)] };
            let act = match current.as_deref().and_then(|c| p.act_record(c, previous.as_deref())) {
                None => no_record(),
                Some(a) => {
                    let mut v = record_segs(a.record, false);
                    if a.source == RecordSource::PreviousAct {
                        v.push(s(" prev", dim().add_modifier(Modifier::ITALIC)));
                    }
                    v
                }
            };
            (rank_segs(content, p.tier, p.rr), peak, act)
        }
    };
    let map = match data.map(|d| &d.maps) {
        _ if hidden => no_record(),
        None | Some(Load::Loading) => loading(),
        Some(Load::Missing | Load::Private) => no_record(),
        Some(Load::Ready(maps)) => {
            let name = content.map(&lobby.map_url);
            let r = maps.iter().find(|(n, _)| n.eq_ignore_ascii_case(&name)).map(|(_, r)| *r).unwrap_or_default();
            if r.games() == 0 { no_record() } else { record_segs(r, r.games() < 5) }
        }
    };
    let (kd, hs) = match data.map(|d| &d.recent) {
        _ if hidden => (dash(), dash()),
        None | Some(Load::Loading) => (loading(), loading()),
        Some(Load::Missing | Load::Private) => (dash(), dash()),
        Some(Load::Ready(recent)) => {
            let (kd, hs) = pooled_kd_hs(recent);
            let kd = kd
                .map(|k| {
                    vec![s(
                        format!("{k:.2}"),
                        if k >= 1.15 {
                            good()
                        } else if k < 0.9 {
                            bad()
                        } else {
                            text()
                        },
                    )]
                })
                .unwrap_or_else(|| vec![s("—", dim())]);
            let hs = hs.map(|h| vec![s(format!("{h}%"), text())]).unwrap_or_else(|| vec![s("—", dim())]);
            (kd, hs)
        }
    };
    join(cols, vec![party_cell, agent, name, lvl, rank, peak, act, map, kd, hs])
}

fn lobby_screen(m: &Model, pregame: bool) -> Screen {
    let lobby = m.lobby.as_ref();
    let map = lobby.map(|l| m.content.map(&l.map_url)).unwrap_or_default();
    let queue = lobby.map(|l| queue_name(&l.queue)).unwrap_or_default();
    let title = if pregame { "AGENT SELECT" } else { "IN GAME" };
    let mut right = vec![s(format!("{queue} · {map}"), dim())];
    if pregame {
        if let Some(end) = lobby.and_then(|l| l.phase_ends) {
            let left = end.saturating_duration_since(Instant::now()).as_secs();
            right.push(s(" · ", dim()));
            right.push(s(format!("{}:{:02}", left / 60, left % 60), warn()));
        }
    } else if let Some((a, e)) = m.my_presence().and_then(|p| p.score) {
        right.extend([
            s(" · ", dim()),
            s(a.to_string(), ally().add_modifier(Modifier::BOLD)),
            s(" – ", dim()),
            s(e.to_string(), enemy().add_modifier(Modifier::BOLD)),
        ]);
    }
    let mut rows = Vec::new();
    match lobby {
        None => rows.extend([blank(), vec![s("   Loading match…", dim())].into(), blank()]),
        Some(l) if !l.is_competitive() => rows.extend([
            blank(),
            vec![s(format!("   {} isn't tracked yet — teamdiff only covers competitive matches for now.", queue), dim())].into(),
            blank(),
        ]),
        Some(l) => {
            let cols = lobby_cols(&map);
            let party = party_marks(&m.parties());
            for (side, label, style) in [(Side::Ally, "YOUR TEAM", ally()), (Side::Enemy, "ENEMY TEAM", enemy())] {
                let slots: Vec<&Slot> = l.slots.iter().filter(|s| s.side == side).collect();
                if slots.is_empty() {
                    continue;
                }
                if side == Side::Enemy {
                    rows.push(blank());
                }
                rows.push(vec![s(label, style.add_modifier(Modifier::BOLD))].into());
                if side == Side::Ally {
                    rows.push(last5_header(&cols));
                    rows.push(header(&cols));
                }
                for slot in slots {
                    rows.push(lobby_row(m, l, slot, &cols, pregame, &party));
                }
            }
        }
    }
    Screen { title: vec![s(title, bold())], right, rows, footer: keys(&[("q", "quit")]) }
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
    let content = &m.content;
    let now = Utc::now();
    let me = m.players.get(&m.me);
    let mut rows = vec![blank()];
    let mut line = vec![s("   ", Style::new())];
    line.extend(name_segs(m, &m.me, false, None));
    if let Some(l) = m.my_presence().and_then(|p| p.account_level) {
        line.push(s(format!("   level {l}"), dim()));
    }
    rows.push(line.into());
    rows.push(blank());
    match me.map(|d| &d.profile) {
        Some(Load::Ready(profile)) => {
            let mut r = vec![s("   Rank       ", dim())];
            if profile.tier == 0 {
                r.push(s("Unranked", dim()));
            } else {
                r.push(tier_seg(content, profile.tier));
                r.extend(profile.rr.map(|rr| s(format!("  {rr} RR"), dim())));
            }
            if profile.peak_tier > 0 {
                r.extend([s("      Peak  ", dim()), tier_seg(content, profile.peak_tier)]);
            }
            rows.push(r.into());
            let mut a = vec![s("   This act   ", dim())];
            match act_ids(m).0.and_then(|c| profile.act_record(&c, None)) {
                Some(ar) => {
                    let p = ar.record.pct().unwrap_or(0);
                    a.extend([s(format!("{}-{}", ar.record.wins, ar.record.losses), bold()), s(format!("  {p}%"), pct_style(p))]);
                }
                None => a.push(s("no games yet", dim())),
            }
            rows.push(a.into());
        }
        Some(Load::Missing) => rows.push(vec![s("   Rank       ", dim()), s("not found on tracker.gg", dim())].into()),
        Some(Load::Private) => rows.push(
            vec![s("   Rank       ", dim()), s("your tracker.gg profile is private; sign in at tracker.gg and make it public", dim())]
                .into(),
        ),
        None | Some(Load::Loading) => rows.push(vec![s("   Rank       ", dim()), s(LOADING, dim())].into()),
    }
    let party = party_marks(&m.parties());
    let mut mates: Vec<&String> = party.keys().filter(|p| **p != m.me).collect();
    // A HashMap's order changes from one frame to the next.
    mates.sort();
    for (i, mate) in mates.iter().enumerate() {
        let mut r = vec![s(if i == 0 { "   Party      " } else { "              " }, dim()), party[*mate].clone(), s(" ", Style::new())];
        r.extend(name_segs(m, mate, false, None));
        match m.players.get(*mate).map(|d| &d.profile) {
            Some(Load::Ready(p)) => {
                r.push(s("  ", Style::new()));
                r.extend(rank_segs(content, p.tier, p.rr));
            }
            Some(Load::Missing) => r.push(s("  —", dim())),
            Some(Load::Private) => r.push(s("  private", dim())),
            None | Some(Load::Loading) => {}
        }
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
    let limited = net.rate_limited_until.lock().ok().and_then(|g| *g).filter(|t| *t > Instant::now());
    if started == finished && limited.is_none() {
        return Vec::new();
    }
    let mut t = format!("loading {finished}/{started}");
    if let Some(until) = limited {
        t.push_str(&format!(" · rate limited, retry {}s", until.saturating_duration_since(Instant::now()).as_secs() + 1));
    }
    vec![s("◐ ", warn()), s(t, warn())]
}

fn screen(m: &Model) -> Screen {
    match &m.phase {
        Phase::Waiting(reason) => waiting_screen(reason),
        Phase::Menus => menus_screen(m),
        Phase::Pregame => lobby_screen(m, true),
        Phase::Ingame => lobby_screen(m, false),
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

    pub fn render(m: &Model, w: u16, h: u16) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, m)).unwrap();
        let buf = t.backend().buffer();
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect()
    }

    #[test]
    fn fit_pads_and_truncates_by_display_width() {
        assert_eq!(width(&fit(vec![s("abc", Style::new())], 6, false)), 6);
        let t = fit(vec![s("Charlie#DEMO-and-more", Style::new())], 10, false);
        assert_eq!(width(&t), 10);
        assert!(t.iter().map(|x| x.content.to_string()).collect::<String>().ends_with('…'));
        let wide = fit(vec![s("한국어이름", Style::new())], 6, false);
        assert_eq!(width(&wide), 6, "double-width characters are measured by cells");
    }

    #[test]
    fn map_header_falls_back_for_long_names() {
        assert_eq!(lobby_cols("Ascent")[7].title, "WIN% ON ASCENT");
        assert_eq!(lobby_cols("Corrode")[7].title, "CORRODE WIN%");
    }

    #[test]
    fn players_without_stats_say_why() {
        let mut m = crate::demo::model("in-game");
        m.players.get_mut("neon").unwrap().profile = Load::Missing;
        let lines = render(&m, 120, 22);
        let row = |agent: &str| lines.iter().find(|l| l.contains(agent)).cloned().unwrap_or_default();
        assert!(row("Omen").contains("incognito"), "{}", row("Omen"));
        assert!(row("Neon").contains("not found"), "{}", row("Neon"));
    }

    #[test]
    fn request_errors_show_in_the_footer() {
        let mut m = Model::new(std::sync::Arc::default());
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
        let m = Model::new(std::sync::Arc::default());
        let lines = render(&m, 80, 20);
        assert!(lines.iter().any(|l| l.contains("Terminal too small")));
        assert!(lines.iter().any(|l| l.contains("Widen to 112 columns (now 80)")));
    }
}
