//! Terminal rendering. Every screen is a fixed-width text grid.

use crate::content::Content;
use crate::model::{Load, Lobby, Model, Phase, Pick, PlayerData, Side, Slot};
use crate::riot::NetStats;
use crate::stats::{MatchSummary, Record, RecordSource, pooled_kd_hs};
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
const INNER: usize = WIDTH as usize - 4;

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
const CURSOR_BG: Color = rgb(0x1f, 0x2a, 0x37);
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
    bg: Option<Color>,
}

impl From<Vec<Seg>> for Row {
    fn from(segs: Vec<Seg>) -> Self {
        Row { segs, ..Default::default() }
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

/// Joins cells into a row. The first two columns (cursor and party marker) sit together; the rest
/// are separated by one space.
fn join(cols: &[Col], cells: Vec<Vec<Seg>>) -> Row {
    let mut row = Row::default();
    for (i, (c, cell)) in cols.iter().zip(cells).enumerate() {
        if i > 1 {
            row.segs.push(s(" ", Style::new()));
        }
        row.segs.extend(fit(cell, c.w, c.right));
    }
    row
}

fn header(cols: &[Col]) -> Row {
    join(cols, cols.iter().map(|c| vec![s(c.title.clone(), dim())]).collect())
}

fn center(segs: Vec<Seg>) -> Row {
    let pad = INNER.saturating_sub(width(&segs)) / 2;
    std::iter::once(s(" ".repeat(pad), Style::new())).chain(segs).collect::<Vec<_>>().into()
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

// ---------- UI state ----------
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Main,
    Detail(String),
}

#[derive(Debug, Default)]
pub struct UiState {
    pub view: View,
    pub cursor: usize,
}

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
    let start: usize = cols[..9].iter().enumerate().map(|(i, c)| c.w + usize::from(i > 1)).sum::<usize>() + 1;
    vec![s(" ".repeat(start), Style::new()), s(" LAST 5  ", dim())].into()
}

fn lobby_row(m: &Model, lobby: &Lobby, slot: &Slot, cols: &[Col], pregame: bool, cursor: bool, party: &HashMap<String, Seg>) -> Row {
    let content = &m.content;
    let sel = vec![s(if cursor { "▸" } else { " " }, ally())];
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
            let (kd, hs) = pooled_kd_hs(recent.iter().map(|r| &r.player));
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
    let mut row = join(cols, vec![sel, party_cell, agent, name, lvl, rank, peak, act, map, kd, hs]);
    if cursor {
        row.bg = Some(CURSOR_BG);
    }
    row
}

/// Players the cursor can move over, in screen order.
pub fn selectable(m: &Model) -> Vec<String> {
    match m.phase {
        Phase::Menus => vec![m.me.clone()],
        Phase::Pregame | Phase::Ingame => {
            m.lobby.as_ref().filter(|l| l.is_competitive()).map(|l| l.slots.iter().map(|s| s.puuid.clone()).collect()).unwrap_or_default()
        }
        Phase::Postgame => m
            .postgame
            .as_ref()
            .and_then(|p| p.summary.as_ref())
            .map(|s| postgame_order(m, s).into_iter().map(|p| p.0).collect())
            .unwrap_or_default(),
        Phase::Waiting(_) => Vec::new(),
    }
}

fn lobby_screen(m: &Model, ui: &UiState, pregame: bool) -> Screen {
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
            let order = selectable(m);
            let cursor_id = order.get(ui.cursor).cloned();
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
                    rows.push(lobby_row(m, l, slot, &cols, pregame, cursor_id.as_deref() == Some(&slot.puuid), &party));
                }
            }
        }
    }
    Screen { title: vec![s(title, bold())], right, rows, footer: keys(&[("↑↓", "select"), ("enter", "details"), ("q", "quit")]) }
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
        footer: keys(&[("enter", "profile"), ("q", "quit")]),
    }
}

// ---------- post-game ----------
const FONT: &[(char, [&str; 5])] = &[
    ('V', ["█   █", "█   █", "█   █", " █ █ ", "  █  "]),
    ('I', ["█████", "  █  ", "  █  ", "  █  ", "█████"]),
    ('C', [" ████", "█    ", "█    ", "█    ", " ████"]),
    ('T', ["█████", "  █  ", "  █  ", "  █  ", "  █  "]),
    ('O', [" ███ ", "█   █", "█   █", "█   █", " ███ "]),
    ('R', ["████ ", "█   █", "████ ", "█  █ ", "█   █"]),
    ('Y', ["█   █", " █ █ ", "  █  ", "  █  ", "  █  "]),
    ('D', ["████ ", "█   █", "█   █", "█   █", "████ "]),
    ('E', ["█████", "█    ", "████ ", "█    ", "█████"]),
    ('F', ["█████", "█    ", "████ ", "█    ", "█    "]),
    ('A', [" ███ ", "█   █", "█████", "█   █", "█   █"]),
    ('W', ["█   █", "█   █", "█ █ █", "██ ██", "█   █"]),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Win,
    Loss,
    Draw,
}

/// Riot's flags decide, not the score: a surrender can end a match at a tied score, and a remake has
/// no winner.
fn outcome(summary: &MatchSummary, team: &str) -> Outcome {
    match summary.teams.iter().find(|(_, (_, won))| *won) {
        Some((t, _)) if t == team => Outcome::Win,
        Some(_) => Outcome::Loss,
        None => Outcome::Draw,
    }
}

/// The block-letter banner. Columns reveal left to right (14 ms each), then a highlight sweeps
/// across every 2.8 s on a win, or the color slowly sags on a loss.
fn banner(word: &str, outcome: Outcome, elapsed_ms: u128) -> Vec<Row> {
    let glyph = |c: char| FONT.iter().find(|(k, _)| *k == c).map(|(_, g)| *g).unwrap_or(["     "; 5]);
    let rows: Vec<String> = (0..5)
        .map(|r| word.chars().map(|c| glyph(c)[r].chars().flat_map(|ch| [ch, ch]).collect::<String>()).collect::<Vec<_>>().join("  "))
        .collect();
    let pad = INNER.saturating_sub(rows[0].chars().count()) / 2;
    let base = match outcome {
        Outcome::Win => (0x4a, 0xde, 0x80),
        Outcome::Loss => (0xf8, 0x71, 0x71),
        Outcome::Draw => (0xc9, 0xd1, 0xd9),
    };
    rows.iter()
        .map(|line| {
            let mut segs = vec![s(" ".repeat(pad), Style::new())];
            for (col, ch) in line.chars().enumerate() {
                let reveal = col as u128 * 14;
                if ch == ' ' || elapsed_ms < reveal {
                    segs.push(s(" ", Style::new()));
                    continue;
                }
                let t = elapsed_ms - reveal;
                let (r, g, b) = match outcome {
                    Outcome::Win if t >= 900 && (t - 900) % 2800 < 196 => (0xf2, 0xff, 0xf6),
                    Outcome::Loss => {
                        let phase = ((t % 3200) as f64 / 3200.0 * std::f64::consts::TAU).cos() * 0.5 + 0.5;
                        let mix = |a: u8, z: u8| (a as f64 * phase + z as f64 * (1.0 - phase)) as u8;
                        (mix(base.0, 0x8f), mix(base.1, 0x3b), mix(base.2, 0x3b))
                    }
                    _ => base,
                };
                segs.push(s(ch.to_string(), fg(rgb(r, g, b))));
            }
            segs.into()
        })
        .collect()
}

fn pg_cols() -> Vec<Col> {
    vec![
        col("", 1, false),
        col("", 1, false),
        col("AGENT", 9, false),
        col("PLAYER", 18, false),
        col("PERF", 4, true),
        col("ACS", 4, true),
        col("K", 3, true),
        col("D", 3, true),
        col("A", 3, true),
        col("+/-", 4, true),
        col("ADR", 4, true),
        col("HS%", 4, true),
        col("FB", 3, true),
    ]
}

/// (puuid, team) for post-game rows: my team first, each team sorted by Performance Score.
fn postgame_order(m: &Model, summary: &MatchSummary) -> Vec<(String, String)> {
    let my_team = summary.player(&m.me).map(|p| p.team.clone()).unwrap_or_default();
    let mut players: Vec<_> = summary.players.iter().collect();
    players.sort_by(|a, b| (a.team != my_team).cmp(&(b.team != my_team)).then(summary.rating(b).total_cmp(&summary.rating(a))));
    players.into_iter().map(|p| (p.puuid.clone(), p.team.clone())).collect()
}

fn postgame_screen(m: &Model, ui: &UiState) -> Screen {
    let footer = keys(&[("↑↓", "select"), ("enter", "details"), ("q", "quit")]);
    let Some(pg) = m.postgame.as_ref() else { return menus_screen(m) };
    let Some(summary) = pg.summary.as_ref() else {
        return Screen {
            title: vec![s("MATCH RESULT", bold())],
            right: Vec::new(),
            rows: vec![blank(), center(vec![s("Waiting for match results…", dim())]), blank()],
            footer,
        };
    };
    let content = &m.content;
    let me = summary.player(&m.me);
    let my_team = me.map(|p| p.team.clone()).unwrap_or_default();
    let (us, them, _) = summary.score_for(&my_team).unwrap_or((0, 0, false));
    let outcome = outcome(summary, &my_team);
    let word = match outcome {
        Outcome::Win => "VICTORY",
        Outcome::Loss => "DEFEAT",
        Outcome::Draw => "DRAW",
    };
    let elapsed = pg.shown_at.map(|t| t.elapsed().as_millis()).unwrap_or(u128::MAX / 2);
    let mins = summary.length_ms / 60_000;
    let right = vec![s(
        format!("{} · {} · {}:{:02}", queue_name(&summary.queue), content.map(&summary.map_url), mins, (summary.length_ms / 1000) % 60),
        dim(),
    )];

    let mut rows = vec![blank()];
    rows.extend(banner(word, outcome, elapsed));
    rows.push(blank());
    rows.push(center(vec![
        s(us.to_string(), ally().add_modifier(Modifier::BOLD)),
        s("  –  ", dim()),
        s(them.to_string(), enemy().add_modifier(Modifier::BOLD)),
    ]));
    if let Some(u) = &pg.rr {
        let tier = content.tier_name(u.tier_after);
        let filled = (u.rr_after.min(100) as usize * 20).div_ceil(100);
        let bar_style = if u.rr_earned >= 0 { good() } else { bad() };
        rows.push(center(vec![
            s(tier.clone(), rank_style(&tier)),
            s(format!("  {} RR  ", u.rr_after), dim()),
            s("▇".repeat(filled), bar_style),
            s("▇".repeat(20 - filled), fg(rgb(0x2a, 0x31, 0x3a))),
            s(format!("  {:+}", u.rr_earned), bar_style.add_modifier(Modifier::BOLD)),
        ]));
    }
    rows.push(blank());

    let cols = pg_cols();
    let lobby = pg.lobby.as_ref();
    let incognito = |p: &str| lobby.and_then(|l| l.slots.iter().find(|s| s.puuid == p)).is_some_and(|s| s.incognito);
    let party = party_marks(&m.parties());
    let order = postgame_order(m, summary);
    let cursor_id = order.get(ui.cursor).map(|p| p.0.clone());
    let mut current_team = None;
    for (puuid, team) in &order {
        if current_team.as_ref() != Some(team) {
            if current_team.is_some() {
                rows.push(blank());
            }
            let (label, style) = if *team == my_team { ("YOUR TEAM", ally()) } else { ("ENEMY TEAM", enemy()) };
            rows.push(vec![s(label, style.add_modifier(Modifier::BOLD))].into());
            if *team == my_team {
                rows.push(header(&cols));
            }
            current_team = Some(team.clone());
        }
        let Some(p) = summary.player(puuid) else { continue };
        let diff = p.kills as i64 - p.deaths as i64;
        let perf = p.performance.map(|v| v.round().to_string()).unwrap_or_else(|| "—".into());
        let cells = vec![
            vec![s(if cursor_id.as_deref() == Some(puuid.as_str()) { "▸" } else { " " }, ally())],
            party.get(puuid).map(|mark| vec![mark.clone()]).unwrap_or_else(|| vec![s(" ", Style::new())]),
            vec![s(content.agent(&p.agent_id), bold())],
            name_segs(m, puuid, incognito(puuid), Some(&p.agent_id)),
            vec![s(perf, bold())],
            vec![s(p.acs().to_string(), text())],
            vec![s(p.kills.to_string(), text())],
            vec![s(p.deaths.to_string(), text())],
            vec![s(p.assists.to_string(), text())],
            vec![s(
                format!("{diff:+}").replace("+0", "0"),
                if diff > 0 {
                    good()
                } else if diff < 0 {
                    bad()
                } else {
                    dim()
                },
            )],
            vec![s(p.adr().to_string(), text())],
            vec![s(p.hs_pct().map(|h| format!("{h}%")).unwrap_or_else(|| "—".into()), text())],
            vec![s(p.first_kills.to_string(), text())],
        ];
        let mut row = join(&cols, cells);
        if cursor_id.as_deref() == Some(puuid.as_str()) {
            row.bg = Some(CURSOR_BG);
        }
        rows.push(row);
    }
    Screen { title: vec![s("MATCH RESULT", bold())], right, rows, footer }
}

// ---------- detail / profile ----------
const HALF: usize = 50;

fn two(left: Row, right: Row) -> Row {
    let mut segs = fit(left.segs, HALF, false);
    segs.push(s("    ", Style::new()));
    segs.extend(right.segs);
    Row { segs, bg: None }
}

fn bar(p: u32) -> Vec<Seg> {
    let f = ((p as f64 / 10.0).round() as usize).min(10);
    vec![s("▇".repeat(f), good()), s("▇".repeat(10 - f), fg(rgb(0x2a, 0x31, 0x3a)))]
}

fn act_rows(m: &Model, d: &PlayerData) -> Vec<Row> {
    let mut rows: Vec<Row> = vec![vec![s("ACT HISTORY", bold())].into(), vec![s("  ACT     RANK         W-L     WIN%", dim())].into()];
    let profile = match &d.profile {
        Load::Ready(p) => p,
        Load::Private => {
            rows.push(vec![s("  private on tracker.gg", dim())].into());
            return rows;
        }
        Load::Missing => {
            rows.push(vec![s("  —", dim())].into());
            return rows;
        }
        Load::Loading => {
            rows.push(vec![s(format!("  {LOADING}"), dim())].into());
            return rows;
        }
    };
    let mut acts: Vec<_> = profile
        .acts
        .iter()
        .chain(&d.history)
        .filter(|a| a.record.games() > 0)
        .filter_map(|a| Some((m.content.act(&a.act_id)?, a)))
        .collect();
    acts.sort_by_key(|(act, _)| std::cmp::Reverse(act.start));
    acts.dedup_by(|a, b| a.0.id == b.0.id);
    for (act, a) in acts.into_iter().take(5) {
        let name = m.content.tier_name(a.tier);
        let p = a.record.pct().unwrap_or(0);
        rows.push(
            vec![
                s(format!("  {:<8}", act.label), text()),
                s(format!("{name:<13}"), rank_style(&name)),
                s(format!("{:<8}", format!("{}-{}", a.record.wins, a.record.losses)), text()),
                s(format!("{p}%"), pct_style(p)),
            ]
            .into(),
        );
    }
    rows
}

fn map_rows(d: &PlayerData, highlight: Option<&str>) -> Vec<Row> {
    let mut rows: Vec<Row> = vec![vec![s("MAPS THIS ACT", bold())].into(), vec![s("  MAP      W-L    WIN%", dim())].into()];
    let maps = match &d.maps {
        Load::Ready(maps) => maps,
        Load::Missing | Load::Private => {
            rows.push(vec![s("  —", dim())].into());
            return rows;
        }
        Load::Loading => {
            rows.push(vec![s(format!("  {LOADING}"), dim())].into());
            return rows;
        }
    };
    for (name, r) in maps {
        let p = r.pct().unwrap_or(0);
        let current = highlight.is_some_and(|h| h.eq_ignore_ascii_case(name));
        let mut row = vec![
            s(format!("  {name:<9}"), if current { bold() } else { text() }),
            s(format!("{:<7}", format!("{}-{}", r.wins, r.losses)), text()),
            s(format!("{:<6}", format!("{p}%")), pct_style(p)),
        ];
        row.extend(bar(p));
        rows.push(row.into());
    }
    rows
}

fn recent_rows(m: &Model, d: &PlayerData) -> Vec<Row> {
    let mut rows: Vec<Row> = vec![
        vec![s("RECENT MATCHES", bold())].into(),
        vec![s("  MAP      AGENT      RESULT    K / D / A   PERF  ACS   HS%", dim())].into(),
    ];
    let recent = match &d.recent {
        Load::Ready(recent) => recent,
        Load::Missing | Load::Private => {
            rows.push(vec![s("  —", dim())].into());
            return rows;
        }
        Load::Loading => {
            rows.push(vec![s(format!("  {LOADING}"), dim())].into());
            return rows;
        }
    };
    for mt in recent {
        let p = &mt.player;
        let (res, style) = match mt.won {
            Some(true) => ("W", good()),
            Some(false) => ("L", bad()),
            None => ("D", dim()),
        };
        rows.push(
            vec![
                s(format!("  {:<9}", m.content.map(&mt.map_id)), text()),
                s(format!("{:<11}", m.content.agent(&p.agent_id)), text()),
                s(res, style),
                s(format!(" {:<8}", format!("{}-{}", mt.rounds_won, mt.rounds_lost)), text()),
                s(format!("{:<12}", format!("{}/{}/{}", p.kills, p.deaths, p.assists)), text()),
                s(format!("{:<6}", p.performance.map(|v| v.round().to_string()).unwrap_or_else(|| "—".into())), bold()),
                s(format!("{:<6}", p.acs()), text()),
                s(p.hs_pct().map(|h| format!("{h}%")).unwrap_or_else(|| "—".into()), text()),
            ]
            .into(),
        );
    }
    rows
}

fn detail_screen(m: &Model, puuid: &str) -> Screen {
    let content = &m.content;
    let lobby = m.lobby.as_ref().or(m.postgame.as_ref().and_then(|p| p.lobby.as_ref()));
    let slot = lobby.and_then(|l| l.slots.iter().find(|s| s.puuid == puuid));
    let incognito = slot.is_some_and(|s| s.incognito) && puuid != m.me;
    // Incognito players are never looked up.
    let d = if incognito {
        PlayerData { profile: Load::Missing, maps: Load::Missing, recent: Load::Missing, ..Default::default() }
    } else {
        m.players.get(puuid).cloned().unwrap_or_default()
    };
    let agent = slot.and_then(|s| s.agent_id.clone());
    let title = if incognito {
        agent.as_deref().map(|a| content.agent(a)).unwrap_or_else(|| "Player".into())
    } else {
        d.name.as_ref().map(|(n, t)| format!("{n}#{t}")).unwrap_or_else(|| if puuid == m.me { "you".into() } else { LOADING.into() })
    };
    let mut head: Vec<Seg> = Vec::new();
    if let Some(a) = &agent {
        head.extend([s(content.agent(a), bold()), s("   ", Style::new())]);
    }
    if let Some(p) = d.profile.ready() {
        head.extend(rank_segs(content, p.tier, p.rr));
        if p.peak_tier > 0 {
            head.extend([s("   peak ", dim()), tier_seg(content, p.peak_tier)]);
        }
    }
    if let Some(l) = slot.filter(|s| !s.hide_level).and_then(|s| s.level) {
        head.push(s(format!("   level {l}"), dim()));
    }
    let parties = m.parties();
    if let Some(party) = parties.get(puuid) {
        let mut mates: Vec<String> = parties
            .iter()
            .filter(|(p, id)| *id == party && *p != puuid)
            .map(|(p, _)| {
                let mate_slot = lobby.and_then(|l| l.slots.iter().find(|s| &s.puuid == p));
                name_segs(m, p, mate_slot.is_some_and(|s| s.incognito), mate_slot.and_then(|s| s.agent_id.as_deref()))
                    .iter()
                    .map(|x| x.content.to_string())
                    .collect()
            })
            .collect();
        mates.sort();
        if !mates.is_empty() {
            head.extend([s("   partied with ", dim()), s(mates.join(", "), text())]);
        }
    }
    let map = m.lobby.as_ref().map(|l| content.map(&l.map_url));
    let (left, right) = (act_rows(m, &d), map_rows(&d, map.as_deref()));
    let mut rows = vec![head.into(), blank()];
    let n = left.len().max(right.len());
    let (mut left, mut right) = (left.into_iter(), right.into_iter());
    for _ in 0..n {
        rows.push(two(left.next().unwrap_or_default(), right.next().unwrap_or_default()));
    }
    rows.push(blank());
    rows.extend(recent_rows(m, &d));
    Screen { title: vec![s(title, bold())], right: Vec::new(), rows, footer: keys(&[("esc", "back"), ("q", "quit")]) }
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

fn screen(m: &Model, ui: &UiState) -> Screen {
    match (&ui.view, &m.phase) {
        (View::Detail(p), _) => detail_screen(m, p),
        (_, Phase::Waiting(reason)) => waiting_screen(reason),
        (_, Phase::Menus) => menus_screen(m),
        (_, Phase::Pregame) => lobby_screen(m, ui, true),
        (_, Phase::Ingame) => lobby_screen(m, ui, false),
        (_, Phase::Postgame) => postgame_screen(m, ui),
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

pub fn draw(f: &mut Frame, m: &Model, ui: &UiState) {
    let sc = screen(m, ui);
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
    let lines: Vec<Line> = sc
        .rows
        .iter()
        .map(|r| {
            let line = Line::from(r.segs.clone());
            match r.bg {
                Some(bg) => line.style(Style::new().bg(bg)),
                None => line,
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines).style(text()).block(block), rect);
}

/// True when the screen animates and needs frequent redraws.
pub fn animating(m: &Model) -> bool {
    m.phase == Phase::Postgame
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    pub fn render(m: &Model, ui: &UiState, w: u16, h: u16) -> Vec<String> {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, m, ui)).unwrap();
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
        assert_eq!(lobby_cols("Ascent")[8].title, "WIN% ON ASCENT");
        assert_eq!(lobby_cols("Corrode")[8].title, "CORRODE WIN%");
    }

    #[test]
    fn banner_reveals_left_to_right() {
        let at = |ms| banner("VICTORY", Outcome::Win, ms).iter().map(|r| width(&r.segs)).max().unwrap();
        assert_eq!(at(0), at(10_000), "rows keep their width while revealing");
        let visible = |ms| banner("VICTORY", Outcome::Win, ms).iter().flat_map(|r| r.segs.clone()).filter(|x| x.content == "█").count();
        assert!(visible(0) < visible(300));
        assert!(visible(300) < visible(5_000));
    }

    #[test]
    fn players_without_stats_say_why() {
        let mut m = crate::demo::model("in-game");
        m.players.get_mut("neon").unwrap().profile = Load::Missing;
        let lines = render(&m, &UiState::default(), 120, 22);
        let row = |agent: &str| lines.iter().find(|l| l.contains(agent)).cloned().unwrap_or_default();
        assert!(row("Omen").contains("incognito"), "{}", row("Omen"));
        assert!(row("Neon").contains("not found"), "{}", row("Neon"));
    }

    #[test]
    fn request_errors_show_in_the_footer() {
        let mut m = Model::new(std::sync::Arc::default());
        let err = "500 Internal Server Error from /name-service/v2/players, with a reason long enough to be cut off".to_string();
        m.last_error = Some((err.clone(), Instant::now()));
        let lines = render(&m, &UiState::default(), WIDTH, 10);
        let footer = lines.iter().find(|l| l.starts_with('└')).unwrap();
        assert!(footer.contains("⚠ 500 Internal Server Error from /name-service/v2/players"), "the status survives truncation: {footer}");
        m.last_error = Some((err, Instant::now() - ERROR_SHOWN_FOR));
        let lines = render(&m, &UiState::default(), WIDTH, 10);
        assert!(!lines.iter().any(|l| l.contains('⚠')), "old errors fade out");
    }

    #[test]
    fn too_small_screen_reports_width() {
        let m = Model::new(std::sync::Arc::default());
        let lines = render(&m, &UiState::default(), 80, 20);
        assert!(lines.iter().any(|l| l.contains("Terminal too small")));
        assert!(lines.iter().any(|l| l.contains("Widen to 112 columns (now 80)")));
    }

    #[test]
    fn riots_flags_decide_the_outcome() {
        let m = crate::demo::model("defeat");
        let mut s = (**m.postgame.as_ref().unwrap().summary.as_ref().unwrap()).clone();
        let mine = s.player(&m.me).unwrap().team.clone();
        for (rounds, _) in s.teams.values_mut() {
            *rounds = 6;
        }
        assert!(outcome(&s, &mine) == Outcome::Loss, "a surrender at a tied score");
        for (_, won) in s.teams.values_mut() {
            *won = false;
        }
        assert!(outcome(&s, &mine) == Outcome::Draw, "no winner");
        s.teams.get_mut(&mine).unwrap().1 = true;
        assert!(outcome(&s, &mine) == Outcome::Win);
    }
}
