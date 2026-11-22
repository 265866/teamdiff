//! Terminal rendering. Every screen is a fixed-width text grid.

use std::cmp::{self, Reverse};
use std::collections::HashMap;
use std::f64::consts::TAU;
use std::iter;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};
use teamdiff_core::content::Content;
use teamdiff_core::goldstars::{self, ALL_TIME, Accolade};
use teamdiff_core::model::{
    LastError, Load, Lobby, Model, Phase, Pick, PlayerData, Postgame, RiotId, Side, Slot, Updated,
};
use teamdiff_core::net_stats::NetStats;
use teamdiff_core::stats::{self, MatchSummary, PooledStats, Record, RecordSource};
use unicode_width::UnicodeWidthStr;

use crate::cast;
use crate::icons::Icons;

/// Every screen is this many columns wide; narrower terminals get the too-small screen.
pub(crate) const WIDTH: u16 = 112;
const INNER: usize = WIDTH as usize - 4;

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
fn gold() -> Style {
    warn().add_modifier(Modifier::BOLD)
}
const CURSOR_BG: Color = rgb(0x1f, 0x2a, 0x37);
const PARTY: [Color; 4] = [
    rgb(0x60, 0xa5, 0xfa),
    rgb(0xe8, 0x79, 0xf9),
    rgb(0xfb, 0x92, 0x3c),
    rgb(0xa3, 0xe6, 0x35),
];

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

type Seg = Span<'static>;

#[cfg_attr(
    dylint_lib = "trellis_lints",
    expect(
        impl_into_params,
        reason = "call-site ergonomics are paramount: every screen builds its text from hundreds \
                  of these calls, half with string literals and half with formatted Strings"
    )
)]
fn s(t: impl Into<String>, style: Style) -> Seg {
    Span::styled(t.into(), style)
}

fn width(segs: &[Seg]) -> usize {
    segs.iter().map(|s| s.content.width()).sum()
}

/// An accolade icon placed at a column offset.
#[derive(Clone, Copy)]
struct IconAt {
    column: usize,
    kind: Accolade,
}

/// A cell's content plus accolade icons placed at column offsets within it.
#[derive(Default, Clone)]
struct Cell {
    segs: Vec<Seg>,
    icons: Vec<IconAt>,
}

impl Cell {
    fn from_segs(segs: Vec<Seg>) -> Self {
        Cell {
            segs,
            icons: Vec::new(),
        }
    }
}

impl From<Vec<Seg>> for Cell {
    fn from(segs: Vec<Seg>) -> Self {
        Self::from_segs(segs)
    }
}

/// One line of a screen, with its accolade icons and its background.
#[derive(Default, Clone)]
struct Row {
    segs: Vec<Seg>,
    icons: Vec<IconAt>,
    bg: Option<Color>,
}

impl Row {
    fn from_segs(segs: Vec<Seg>) -> Self {
        Row {
            segs,
            ..Default::default()
        }
    }
}

impl From<Vec<Seg>> for Row {
    fn from(segs: Vec<Seg>) -> Self {
        Self::from_segs(segs)
    }
}

fn blank() -> Row {
    Row::default()
}

/// Which side of a column its text keeps to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Right,
}

/// Truncates with an ellipsis or pads to exactly `w` columns.
fn fit(segs: Vec<Seg>, w: usize, align: Align) -> Vec<Seg> {
    use unicode_width::UnicodeWidthChar;

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
                let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
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
    match align {
        Align::Right => iter::once(pad).chain(segs).collect(),
        Align::Left => segs.into_iter().chain(iter::once(pad)).collect(),
    }
}

struct Col {
    title: String,
    w: usize,
    align: Align,
}

fn col(title: &str, w: usize, align: Align) -> Col {
    Col {
        title: title.into(),
        w,
        align,
    }
}

/// Joins cells into a row. The first two columns (cursor and party marker) sit together; the rest
/// are separated by one space.
fn join(cols: &[Col], cells: Vec<Cell>) -> Row {
    let mut row = Row::default();
    let mut x = 0;
    for (i, (c, cell)) in cols.iter().zip(cells).enumerate() {
        if i > 1 {
            row.segs.push(s(" ", Style::new()));
            x += 1;
        }
        row.icons.extend(cell.icons.iter().map(|icon| IconAt {
            column: x + icon.column,
            kind: icon.kind,
        }));
        row.segs.extend(fit(cell.segs, c.w, c.align));
        x += c.w;
    }
    row
}

fn header(cols: &[Col]) -> Row {
    join(
        cols,
        cols.iter()
            .map(|c| vec![s(c.title.clone(), dim())].into())
            .collect(),
    )
}

fn center(segs: Vec<Seg>) -> Row {
    let pad = INNER.saturating_sub(width(&segs)) / 2;
    iter::once(s(" ".repeat(pad), Style::new()))
        .chain(segs)
        .collect::<Vec<_>>()
        .into()
}

/// A key and what it does, for the footer.
struct KeyHint {
    key: &'static str,
    action: &'static str,
}

const fn hint(key: &'static str, action: &'static str) -> KeyHint {
    KeyHint { key, action }
}

fn keys(hints: &[KeyHint]) -> Vec<Seg> {
    hints
        .iter()
        .enumerate()
        .flat_map(|(i, h)| {
            [
                s(
                    format!("{}{}", if i > 0 { "  " } else { "" }, h.key),
                    bold(),
                ),
                s(format!(" {}", h.action), dim()),
            ]
        })
        .collect()
}

const LOADING: &str = "···";
const ERROR_SHOWN_FOR: Duration = Duration::from_secs(15);
const UPDATED_SHOWN_FOR: Duration = Duration::from_secs(10);
/// Every release, newest first, with what each changed.
const RELEASES: &str = "https://github.com/265866/teamdiff/releases";

/// Which screen shows: the phase's own, or one player's details.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum View {
    /// The phase's own screen.
    #[default]
    Main,
    /// The details of the player with this PUUID.
    Detail(String),
}

/// What the keys have changed: the screen, the selected player and the legend.
#[derive(Debug, Default)]
pub(crate) struct UiState {
    /// The screen that shows.
    pub(crate) view: View,
    /// The selected player's index in [`selectable`].
    pub(crate) cursor: usize,
    /// Whether the accolade legend shows over the screen.
    pub(crate) legend: bool,
}

/// A screen ready to draw.
struct Screen {
    title: Vec<Seg>,
    right: Vec<Seg>,
    rows: Vec<Row>,
    footer: Vec<Seg>,
}

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
            c.next()
                .map_or_default(|f| f.to_uppercase().collect::<String>() + c.as_str())
        }
    }
}

/// Each player's party marker, one color per party.
fn party_marks(parties: &HashMap<String, String>) -> HashMap<String, Seg> {
    let mut ids: Vec<&String> = parties.values().collect();
    ids.sort();
    ids.dedup();
    let color: HashMap<&String, Color> = ids
        .into_iter()
        .enumerate()
        .map(|(i, id)| (id, PARTY[i % PARTY.len()]))
        .collect();
    parties
        .iter()
        .map(|(p, id)| (p.clone(), s("■", fg(color[id]))))
        .collect()
}

/// Whether a player hides their name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Named,
    Incognito,
}

impl Visibility {
    fn of(incognito: bool) -> Self {
        if incognito {
            Self::Incognito
        } else {
            Self::Named
        }
    }
}

/// The player's name. The game hides an incognito player's name from others, never from
/// themselves.
fn name_segs(m: &Model, puuid: &str, visibility: Visibility, agent: Option<&str>) -> Vec<Seg> {
    if visibility == Visibility::Incognito && puuid != m.me {
        return match agent {
            Some(a) => vec![s(m.content.agent(a), text().add_modifier(Modifier::ITALIC))],
            None => vec![s("Player", dim().add_modifier(Modifier::ITALIC))],
        };
    }
    let name = m.players.get(puuid).and_then(|d| d.name.clone());
    match (name, puuid == m.me) {
        (Some(RiotId { name, .. }), true) => vec![s(name, you())],
        (None, true) => vec![s("you", you())],
        (Some(RiotId { name, tag }), false) => vec![s(name, text()), s(format!("#{tag}"), dim())],
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

/// How a record's win percentage is colored.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shade {
    /// Green when high, red when low.
    ByPercentage,
    /// Dim, for too few games to go by.
    Dim,
}

fn record_segs(r: Record, shade: Shade) -> Vec<Seg> {
    let Some(p) = r.pct() else {
        return vec![s("  —", dim())];
    };
    let style = match shade {
        Shade::ByPercentage => pct_style(p),
        Shade::Dim => dim(),
    };
    vec![
        s(format!("{p:>3}%"), style),
        s(format!(" {}-{}", r.wins, r.losses), dim()),
    ]
}

/// The current act's id and the one before it.
struct ActIds {
    current: Option<String>,
    previous: Option<String>,
}

fn act_ids(m: &Model) -> ActIds {
    let current = m.content.current_act(Timestamp::now());
    let previous = current
        .and_then(|a| m.content.previous_act(&a.id))
        .map(|a| a.id.clone());
    ActIds {
        current: current.map(|a| a.id.clone()),
        previous,
    }
}

fn lobby_cols(map: &str) -> Vec<Col> {
    let upper = map.to_uppercase();
    let map_title = if upper.len() + 8 <= 14 {
        format!("WIN% ON {upper}")
    } else {
        format!("{upper} WIN%")
    };
    vec![
        col("", 1, Align::Left),
        col("", 1, Align::Left),
        col("AGENT", 11, Align::Left),
        col("PLAYER", 18, Align::Left),
        col("LVL", 4, Align::Right),
        col("RANK", 16, Align::Left),
        col("PEAK", 11, Align::Left),
        col("WIN% THIS ACT", 15, Align::Left),
        col(&map_title, 14, Align::Left),
        col("K/D", 4, Align::Right),
        col("HS%", 4, Align::Right),
    ]
}

fn last5_header(cols: &[Col]) -> Row {
    let start: usize = cols[..9]
        .iter()
        .enumerate()
        .map(|(i, c)| c.w + usize::from(i > 1))
        .sum::<usize>()
        + 1;
    vec![s(" ".repeat(start), Style::new()), s(" LAST 5  ", dim())].into()
}

/// Which lobby screen shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    AgentSelect,
    InGame,
}

fn loading() -> Vec<Seg> {
    vec![s(LOADING, dim())]
}

fn dash() -> Vec<Seg> {
    vec![s("—", dim())]
}

fn no_record() -> Vec<Seg> {
    vec![s("  —", dim())]
}

/// The rank, peak and act record cells of a lobby row.
struct ProfileCells {
    rank: Vec<Seg>,
    peak: Vec<Seg>,
    act: Vec<Seg>,
}

/// The rank, peak and act record cells for a player's data. A `hidden` player, incognito and not
/// you, is never looked up, so they have no stats.
fn profile_cells(m: &Model, data: Option<&PlayerData>, hidden: bool) -> ProfileCells {
    let content = &m.content;
    let ActIds { current, previous } = act_ids(m);
    let (rank, peak, act) = match data.map(|d| &d.profile) {
        _ if hidden => (vec![s("incognito", dim())], dash(), no_record()),
        None | Some(Load::Loading) => (loading(), loading(), loading()),
        Some(Load::Missing) => (vec![s("not found", dim())], dash(), no_record()),
        Some(Load::Private) => (vec![s("private", dim())], dash(), no_record()),
        Some(Load::Ready(p)) => {
            let peak = if p.peak_tier == 0 {
                dash()
            } else {
                vec![tier_seg(content, p.peak_tier)]
            };
            let act = match current
                .as_deref()
                .and_then(|c| p.act_record(c, previous.as_deref()))
            {
                None => no_record(),
                Some(a) => {
                    let mut v = record_segs(a.record, Shade::ByPercentage);
                    if a.source == RecordSource::PreviousAct {
                        v.push(s(" prev", dim().add_modifier(Modifier::ITALIC)));
                    }
                    v
                }
            };
            (rank_segs(content, p.tier, p.rr), peak, act)
        }
    };
    ProfileCells { rank, peak, act }
}

/// The lobby's map record cell for a player's data, dim under five games.
fn map_cell(m: &Model, lobby: &Lobby, data: Option<&PlayerData>, hidden: bool) -> Vec<Seg> {
    match data.map(|d| &d.maps) {
        _ if hidden => no_record(),
        None | Some(Load::Loading) => loading(),
        Some(Load::Missing | Load::Private) => no_record(),
        Some(Load::Ready(maps)) => {
            let name = m.content.map(&lobby.map_url);
            let r = maps
                .iter()
                .find(|m| m.name.eq_ignore_ascii_case(&name))
                .map_or_default(|m| m.record);
            if r.games() == 0 {
                no_record()
            } else {
                record_segs(
                    r,
                    if r.games() < 5 {
                        Shade::Dim
                    } else {
                        Shade::ByPercentage
                    },
                )
            }
        }
    }
}

/// The K/D and HS% cells of a lobby row.
struct RecentCells {
    kd: Vec<Seg>,
    hs: Vec<Seg>,
}

/// The K/D and HS% cells for a player's data, pooled over their recent matches.
fn recent_cells(data: Option<&PlayerData>, hidden: bool) -> RecentCells {
    let (kd, hs) = match data.map(|d| &d.recent) {
        _ if hidden => (dash(), dash()),
        None | Some(Load::Loading) => (loading(), loading()),
        Some(Load::Missing | Load::Private) => (dash(), dash()),
        Some(Load::Ready(recent)) => {
            let PooledStats { kd, hs_pct, .. } =
                stats::pooled_kd_hs(recent.iter().map(|r| &r.player));
            let kd = kd.map_or_else(dash, |k| {
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
            });
            let hs = hs_pct.map_or_else(dash, |h| vec![s(format!("{h}%"), text())]);
            (kd, hs)
        }
    };
    RecentCells { kd, hs }
}

fn lobby_row(
    m: &Model,
    lobby: &Lobby,
    slot: &Slot,
    cols: &[Col],
    stage: Stage,
    cursor: bool,
    party: &HashMap<String, Seg>,
) -> Row {
    let pregame = stage == Stage::AgentSelect;
    let sel = vec![s(if cursor { "▸" } else { " " }, ally())];
    let party_cell = match party.get(&slot.puuid) {
        Some(mark) => vec![mark.clone()],
        None => vec![s(" ", Style::new())],
    };
    let agent_name = slot.agent_id.as_deref().map(|a| m.content.agent(a));
    let agent = match &agent_name {
        Some(a) if !pregame => vec![s(a.clone(), bold())],
        Some(a) if slot.pick == Pick::Locked => vec![s("◆ ", dim()), s(a.clone(), bold())],
        Some(a) if slot.pick == Pick::Hover => vec![
            s("◇ ", dim()),
            s(a.clone(), dim().add_modifier(Modifier::ITALIC)),
        ],
        _ => vec![s("  —", dim())],
    };
    let shown_agent = if pregame && slot.pick == Pick::None {
        None
    } else {
        slot.agent_id.as_deref()
    };
    let name = name_segs(m, &slot.puuid, Visibility::of(slot.incognito), shown_agent);
    let lvl = match (slot.hide_level, slot.level) {
        (false, Some(l)) => vec![s(l.to_string(), text())],
        _ => dash(),
    };
    let data = m.players.get(&slot.puuid);
    let hidden = slot.incognito && slot.puuid != m.me;
    let ProfileCells { rank, peak, act } = profile_cells(m, data, hidden);
    let map = map_cell(m, lobby, data, hidden);
    let RecentCells { kd, hs } = recent_cells(data, hidden);
    let mut row = join(
        cols,
        [
            sel, party_cell, agent, name, lvl, rank, peak, act, map, kd, hs,
        ]
        .into_iter()
        .map(Cell::from)
        .collect(),
    );
    if cursor {
        row.bg = Some(CURSOR_BG);
    }
    row
}

/// Players the cursor can move over, in screen order.
pub(crate) fn selectable(m: &Model) -> Vec<String> {
    match m.phase {
        Phase::Menus => vec![m.me.clone()],
        Phase::Pregame | Phase::Ingame => m
            .lobby
            .as_ref()
            .filter(|l| l.is_competitive())
            .map_or_default(|l| l.slots.iter().map(|s| s.puuid.clone()).collect()),
        Phase::Postgame => m
            .postgame
            .as_ref()
            .and_then(|p| p.summary.as_ref())
            .map_or_default(|s| postgame_order(m, s).into_iter().map(|p| p.puuid).collect()),
        Phase::Waiting(_) => Vec::new(),
    }
}

fn lobby_screen(m: &Model, ui: &UiState, stage: Stage) -> Screen {
    let lobby = m.lobby.as_ref();
    let map = lobby.map_or_default(|l| m.content.map(&l.map_url));
    let queue = lobby.map_or_default(|l| queue_name(&l.queue));
    let title = match stage {
        Stage::AgentSelect => "AGENT SELECT",
        Stage::InGame => "IN GAME",
    };
    let mut right = vec![s(format!("{queue} · {map}"), dim())];
    if stage == Stage::AgentSelect {
        if let Some(end) = lobby.and_then(|l| l.phase_ends) {
            let left = end.saturating_duration_since(Instant::now()).as_secs();
            right.push(s(" · ", dim()));
            right.push(s(format!("{}:{:02}", left / 60, left % 60), warn()));
        }
    } else if let Some(score) = m.my_presence().and_then(|p| p.score) {
        right.extend([
            s(" · ", dim()),
            s(score.ally.to_string(), ally().add_modifier(Modifier::BOLD)),
            s(" – ", dim()),
            s(
                score.enemy.to_string(),
                enemy().add_modifier(Modifier::BOLD),
            ),
        ]);
    }
    let mut rows = Vec::new();
    match lobby {
        None => rows.extend([blank(), vec![s("   Loading match…", dim())].into(), blank()]),
        Some(l) if !l.is_competitive() => rows.extend([
            blank(),
            vec![s(
                format!(
                    "   {queue} isn't tracked yet — teamdiff only covers competitive matches for \
                     now."
                ),
                dim(),
            )]
            .into(),
            blank(),
        ]),
        Some(l) => {
            let cols = lobby_cols(&map);
            let party = party_marks(&m.parties());
            let order = selectable(m);
            let cursor_id = order.get(ui.cursor).cloned();
            for (side, label, style) in [
                (Side::Ally, "YOUR TEAM", ally()),
                (Side::Enemy, "ENEMY TEAM", enemy()),
            ] {
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
                    let cursor = cursor_id.as_deref() == Some(&slot.puuid);
                    rows.push(lobby_row(m, l, slot, &cols, stage, cursor, &party));
                }
            }
        }
    }
    Screen {
        title: vec![s(title, bold())],
        right,
        rows,
        footer: keys(&[
            hint("↑↓", "select"),
            hint("enter", "details"),
            hint("?", "legend"),
            hint("q", "quit"),
        ]),
    }
}

fn waiting_screen(reason: &str) -> Screen {
    Screen {
        title: vec![s("teamdiff", bold())],
        right: vec![s("○ offline", dim())],
        rows: vec![
            blank(),
            vec![s("   Waiting for VALORANT…", bold())].into(),
            vec![s(format!("   {reason}"), dim())].into(),
            blank(),
        ],
        footer: keys(&[hint("q", "quit")]),
    }
}

/// When you entered the queue, from presence's `queueEntryTime`, such as `2025.01.02-03.04.05`,
/// in UTC.
fn parse_queue_time(t: &str) -> Option<Timestamp> {
    let time = DateTime::strptime("%Y.%m.%d-%H.%M.%S", t).ok()?;
    TimeZone::UTC.to_timestamp(time).ok()
}

fn menus_screen(m: &Model) -> Screen {
    let content = &m.content;
    let now = Timestamp::now();
    let me = m.players.get(&m.me);
    let mut rows = vec![blank()];
    let mut line = vec![s("   ", Style::new())];
    line.extend(name_segs(m, &m.me, Visibility::Named, None));
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
                r.extend([
                    s("      Peak  ", dim()),
                    tier_seg(content, profile.peak_tier),
                ]);
            }
            rows.push(r.into());
            let mut a = vec![s("   This act   ", dim())];
            match act_ids(m)
                .current
                .and_then(|c| profile.act_record(&c, None))
            {
                Some(ar) => {
                    let p = ar.record.pct().unwrap_or(0);
                    a.extend([
                        s(format!("{}-{}", ar.record.wins, ar.record.losses), bold()),
                        s(format!("  {p}%"), pct_style(p)),
                    ]);
                }
                None => a.push(s("no games yet", dim())),
            }
            rows.push(a.into());
        }
        Some(Load::Missing) => rows.push(
            vec![
                s("   Rank       ", dim()),
                s("not found on tracker.gg", dim()),
            ]
            .into(),
        ),
        Some(Load::Private) => rows.push(
            vec![
                s("   Rank       ", dim()),
                s(
                    "your tracker.gg profile is private; sign in at tracker.gg and make it public",
                    dim(),
                ),
            ]
            .into(),
        ),
        None | Some(Load::Loading) => {
            rows.push(vec![s("   Rank       ", dim()), s(LOADING, dim())].into());
        }
    }
    rows.extend(party_rows(m));
    let p = m.my_presence();
    let queue = p
        .and_then(|p| p.queue_id.clone())
        .map_or_else(|| "—".into(), |q| queue_name(&q));
    let status = match p.and_then(|p| p.party_state.clone()).as_deref() {
        Some("MATCHMAKING") => {
            let waited = p
                .and_then(|p| p.queue_entry_time.as_deref())
                .and_then(parse_queue_time)
                .map(|t| now.duration_since(t).as_secs().max(0));
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
        footer: keys(&[
            hint("enter", "profile"),
            hint("?", "legend"),
            hint("q", "quit"),
        ]),
    }
}

/// Your party on the menus screen, sorted, since a `HashMap`'s order changes from one frame to
/// the next.
fn party_rows(m: &Model) -> Vec<Row> {
    let party = party_marks(&m.parties());
    let mut mates: Vec<&String> = party.keys().filter(|p| **p != m.me).collect();
    mates.sort();
    let mut rows = Vec::new();
    for (i, mate) in mates.iter().enumerate() {
        let mut r = vec![
            s(
                if i == 0 {
                    "   Party      "
                } else {
                    "              "
                },
                dim(),
            ),
            party[*mate].clone(),
            s(" ", Style::new()),
        ];
        r.extend(name_segs(m, mate, Visibility::Named, None));
        match m.players.get(*mate).map(|d| &d.profile) {
            Some(Load::Ready(p)) => {
                r.push(s("  ", Style::new()));
                r.extend(rank_segs(&m.content, p.tier, p.rr));
            }
            Some(Load::Missing) => r.push(s("  —", dim())),
            Some(Load::Private) => r.push(s("  private", dim())),
            None | Some(Load::Loading) => {}
        }
        rows.push(r.into());
    }
    rows
}

/// The block letters of the banner, five rows each.
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Win,
    Loss,
    Draw,
}

/// Riot's flags decide, not the score: a surrender can end a match at a tied score, and a remake has
/// no winner.
fn outcome(summary: &MatchSummary, team: &str) -> Outcome {
    match summary.teams.iter().find(|(_, result)| result.won) {
        Some((t, _)) if t == team => Outcome::Win,
        Some(_) => Outcome::Loss,
        None => Outcome::Draw,
    }
}

/// The block-letter banner. Columns reveal left to right (14 ms each), then a highlight sweeps
/// across every 2.8 s on a win, or the color slowly sags on a loss.
fn banner(word: &str, outcome: Outcome, elapsed_ms: u128) -> Vec<Row> {
    let glyph = |c: char| {
        FONT.iter()
            .find(|(k, _)| *k == c)
            .map_or(["     "; 5], |(_, g)| *g)
    };
    let rows: Vec<String> = (0..5)
        .map(|r| {
            word.chars()
                .map(|c| {
                    glyph(c)[r]
                        .chars()
                        .flat_map(|ch| [ch, ch])
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("  ")
        })
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
                        let phase = (cast::millis_f64(t % 3200) / 3200.0 * TAU).cos() * 0.5 + 0.5;
                        let mix = |a: u8, z: u8| {
                            cast::truncate_u8(f64::from(a) * phase + f64::from(z) * (1.0 - phase))
                        };
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
        col("", 1, Align::Left),
        col("", 1, Align::Left),
        col("AGENT", 9, Align::Left),
        col("PLAYER", 18, Align::Left),
        col("PERF", 4, Align::Right),
        col("ACS", 4, Align::Right),
        col("K", 3, Align::Right),
        col("D", 3, Align::Right),
        col("A", 3, Align::Right),
        col("+/-", 4, Align::Right),
        col("ADR", 4, Align::Right),
        col("HS%", 4, Align::Right),
        col("FB", 3, Align::Right),
        col("", 9, Align::Left),
        col("ACCOLADES", 22, Align::Left),
    ]
}

/// A player's place on the post-game scoreboard.
struct Placed {
    puuid: String,
    team: String,
}

/// The post-game rows: my team first, each team sorted by Performance Score.
fn postgame_order(m: &Model, summary: &MatchSummary) -> Vec<Placed> {
    let my_team = summary.player(&m.me).map_or_default(|p| p.team.clone());
    let mut players: Vec<_> = summary.players.iter().collect();
    players.sort_by(|a, b| {
        (a.team != my_team)
            .cmp(&(b.team != my_team))
            .then(summary.rating(b).total_cmp(&summary.rating(a)))
    });
    players
        .into_iter()
        .map(|p| Placed {
            puuid: p.puuid.clone(),
            team: p.team.clone(),
        })
        .collect()
}

fn postgame_screen(m: &Model, ui: &UiState) -> Screen {
    let footer = keys(&[
        hint("↑↓", "select"),
        hint("enter", "details"),
        hint("?", "legend"),
        hint("q", "quit"),
    ]);
    let Some(pg) = m.postgame.as_ref() else {
        return menus_screen(m);
    };
    let Some(summary) = pg.summary.as_ref() else {
        return Screen {
            title: vec![s("MATCH RESULT", bold())],
            right: Vec::new(),
            rows: vec![
                blank(),
                center(vec![s("Waiting for match results…", dim())]),
                blank(),
            ],
            footer,
        };
    };
    let my_team = summary.player(&m.me).map_or_default(|p| p.team.clone());
    let minutes = summary.length_ms / 60_000;
    let right = vec![s(
        format!(
            "{} · {} · {}:{:02}",
            queue_name(&summary.queue),
            m.content.map(&summary.map_url),
            minutes,
            (summary.length_ms / 1000) % 60
        ),
        dim(),
    )];
    let mut rows = postgame_head(m, pg, summary, &my_team);
    rows.extend(scoreboard(m, ui, pg, summary, &my_team));
    Screen {
        title: vec![s("MATCH RESULT", bold())],
        right,
        rows,
        footer,
    }
}

/// The banner, the score, your RR change and your new act bests, above the scoreboard.
fn postgame_head(m: &Model, pg: &Postgame, summary: &MatchSummary, my_team: &str) -> Vec<Row> {
    let content = &m.content;
    let score = summary.score_for(my_team).unwrap_or_default();
    let outcome = outcome(summary, my_team);
    let word = match outcome {
        Outcome::Win => "VICTORY",
        Outcome::Loss => "DEFEAT",
        Outcome::Draw => "DRAW",
    };
    let elapsed = pg
        .shown_at
        .map_or(u128::MAX / 2, |t| t.elapsed().as_millis());
    let mut rows = vec![blank()];
    rows.extend(banner(word, outcome, elapsed));
    rows.push(blank());
    rows.push(center(vec![
        s(score.own.to_string(), ally().add_modifier(Modifier::BOLD)),
        s("  –  ", dim()),
        s(
            score.other.to_string(),
            enemy().add_modifier(Modifier::BOLD),
        ),
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
            s(
                format!("  {:+}", u.rr_earned),
                bar_style.add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    let mine = pg.accolades.as_ref().and_then(|a| a.players.get(&m.me));
    let bests: Vec<String> = Accolade::ALL
        .iter()
        .filter_map(|k| {
            mine.and_then(|e| e.get(k))
                .filter(|e| e.act_record)
                .map(|e| {
                    format!(
                        "{} {}",
                        goldstars::format_value(*k, e.value),
                        k.name().to_lowercase()
                    )
                })
        })
        .collect();
    rows.push(blank());
    if !bests.is_empty() {
        rows.push(center(vec![
            s("New act best  ", gold()),
            s(bests.join(" · "), bold()),
        ]));
        rows.push(blank());
    }
    rows
}

/// Both teams' scoreboards, yours first with the column titles.
fn scoreboard(
    m: &Model,
    ui: &UiState,
    pg: &Postgame,
    summary: &MatchSummary,
    my_team: &str,
) -> Vec<Row> {
    let cols = pg_cols();
    let order = postgame_order(m, summary);
    let cursor_id = order.get(ui.cursor).map(|p| p.puuid.clone());
    let party = party_marks(&m.parties());
    let mut rows = Vec::new();
    let mut current_team = None;
    for placed in &order {
        let team = &placed.team;
        if current_team.as_ref() != Some(team) {
            if current_team.is_some() {
                rows.push(blank());
            }
            let (label, style) = if *team == my_team {
                ("YOUR TEAM", ally())
            } else {
                ("ENEMY TEAM", enemy())
            };
            rows.push(vec![s(label, style.add_modifier(Modifier::BOLD))].into());
            if *team == my_team {
                rows.push(header(&cols));
            }
            current_team = Some(team.clone());
        }
        let cursor = cursor_id.as_deref() == Some(placed.puuid.as_str());
        if let Some(row) = postgame_row(m, pg, summary, placed, &cols, cursor, &party) {
            rows.push(row);
        }
    }
    rows
}

/// One player's line on the post-game scoreboard, or `None` for a player the summary lacks.
fn postgame_row(
    m: &Model,
    pg: &Postgame,
    summary: &MatchSummary,
    placed: &Placed,
    cols: &[Col],
    cursor: bool,
    party: &HashMap<String, Seg>,
) -> Option<Row> {
    let content = &m.content;
    let Placed { puuid, team } = placed;
    let p = summary.player(puuid)?;
    let mvp = summary.mvp().map(|p| p.puuid.clone());
    let lobby = pg.lobby.as_ref();
    let incognito = lobby
        .and_then(|l| l.slots.iter().find(|s| s.puuid == *puuid))
        .is_some_and(|s| s.incognito);
    let label = if mvp.as_deref() == Some(puuid.as_str()) {
        Some("MATCH MVP")
    } else if summary.team_best(team).is_some_and(|b| &b.puuid == puuid) {
        Some("TEAM MVP")
    } else {
        None
    };
    let diff = i64::from(p.kills) - i64::from(p.deaths);
    let earned = pg.accolades.as_ref().and_then(|a| a.players.get(puuid));
    let mut acc = Cell::default();
    let earned_icons = Accolade::ICONS
        .iter()
        .filter(|k| earned.is_some_and(|e| e.contains_key(k)));
    for (i, k) in earned_icons.enumerate() {
        acc.icons.push(IconAt {
            column: i * 2,
            kind: *k,
        });
        acc.segs.push(s(k.abbrev(), dim()));
    }
    let perf = p
        .performance
        .map_or_else(|| "—".into(), |v| v.round().to_string());
    let cells: Vec<Cell> = vec![
        vec![s(if cursor { "▸" } else { " " }, ally())].into(),
        party
            .get(puuid)
            .map_or_else(|| vec![s(" ", Style::new())], |mark| vec![mark.clone()])
            .into(),
        vec![s(content.agent(&p.agent_id), bold())].into(),
        name_segs(m, puuid, Visibility::of(incognito), Some(&p.agent_id)).into(),
        vec![s(perf, if label.is_some() { gold() } else { bold() })].into(),
        vec![s(p.acs().to_string(), text())].into(),
        vec![s(p.kills.to_string(), text())].into(),
        vec![s(p.deaths.to_string(), text())].into(),
        vec![s(p.assists.to_string(), text())].into(),
        vec![s(
            format!("{diff:+}").replace("+0", "0"),
            match diff.cmp(&0) {
                cmp::Ordering::Greater => good(),
                cmp::Ordering::Less => bad(),
                cmp::Ordering::Equal => dim(),
            },
        )]
        .into(),
        vec![s(p.adr().to_string(), text())].into(),
        vec![s(
            p.hs_pct().map_or_else(|| "—".into(), |h| format!("{h}%")),
            text(),
        )]
        .into(),
        vec![s(p.first_kills.to_string(), text())].into(),
        vec![s(label.unwrap_or(""), gold())].into(),
        acc,
    ];
    let mut row = join(cols, cells);
    if cursor {
        row.bg = Some(CURSOR_BG);
    }
    Some(row)
}

const HALF: usize = 50;

fn two(left: Row, right: Row) -> Row {
    let mut segs = fit(left.segs, HALF, Align::Left);
    segs.push(s("    ", Style::new()));
    let off = HALF + 4;
    let mut icons = left.icons;
    icons.extend(right.icons.into_iter().map(|icon| IconAt {
        column: icon.column + off,
        kind: icon.kind,
    }));
    segs.extend(right.segs);
    Row {
        segs,
        icons,
        bg: None,
    }
}

fn bar(p: u32) -> Vec<Seg> {
    let f = cast::round_usize(f64::from(p) / 10.0).min(10);
    vec![
        s("▇".repeat(f), good()),
        s("▇".repeat(10 - f), fg(rgb(0x2a, 0x31, 0x3a))),
    ]
}

fn act_rows(m: &Model, d: &PlayerData) -> Vec<Row> {
    let mut rows: Vec<Row> = vec![
        vec![s("ACT HISTORY", bold())].into(),
        vec![s("  ACT     RANK         W-L     WIN%", dim())].into(),
    ];
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
    acts.sort_by_key(|(act, _)| Reverse(act.start));
    acts.dedup_by(|a, b| a.0.id == b.0.id);
    for (act, a) in acts.into_iter().take(5) {
        let name = m.content.tier_name(a.tier);
        let p = a.record.pct().unwrap_or(0);
        rows.push(
            vec![
                s(format!("  {:<8}", act.label), text()),
                s(format!("{name:<13}"), rank_style(&name)),
                s(
                    format!("{:<8}", format!("{}-{}", a.record.wins, a.record.losses)),
                    text(),
                ),
                s(format!("{p}%"), pct_style(p)),
            ]
            .into(),
        );
    }
    rows
}

fn map_rows(d: &PlayerData, highlight: Option<&str>) -> Vec<Row> {
    let mut rows: Vec<Row> = vec![
        vec![s("MAPS THIS ACT", bold())].into(),
        vec![s("  MAP      W-L    WIN%", dim())].into(),
    ];
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
    for map in maps {
        let (name, r) = (&map.name, map.record);
        let p = r.pct().unwrap_or(0);
        let current = highlight.is_some_and(|h| h.eq_ignore_ascii_case(name));
        let mut row = vec![
            s(
                format!("  {name:<9}"),
                if current { bold() } else { text() },
            ),
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
        vec![s(
            "  MAP      AGENT      RESULT    K / D / A   PERF  ACS   HS%",
            dim(),
        )]
        .into(),
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
                s(
                    format!(" {:<8}", format!("{}-{}", mt.rounds_won, mt.rounds_lost)),
                    text(),
                ),
                s(
                    format!("{:<12}", format!("{}/{}/{}", p.kills, p.deaths, p.assists)),
                    text(),
                ),
                s(
                    format!(
                        "{:<6}",
                        p.performance
                            .map_or_else(|| "—".into(), |v| v.round().to_string())
                    ),
                    bold(),
                ),
                s(format!("{:<6}", p.acs()), text()),
                s(
                    p.hs_pct().map_or_else(|| "—".into(), |h| format!("{h}%")),
                    text(),
                ),
            ]
            .into(),
        );
    }
    rows
}

fn accolade_rows(m: &Model) -> Vec<Row> {
    let mut rows: Vec<Row> = vec![
        vec![s("ACCOLADES", bold())].into(),
        vec![s(format!("{:17}{:<14}ALL-TIME", "", "THIS ACT"), dim())].into(),
        vec![s(format!("{:17}times best    times best", ""), dim())].into(),
    ];
    let Some(g) = &m.my_goldstars else {
        rows.push(vec![s(format!("  {LOADING}"), dim())].into());
        return rows;
    };
    let act = act_ids(m).current.unwrap_or_default();
    let latest = g
        .matches
        .iter()
        .filter(|x| x.players.contains_key(&m.me))
        .max_by_key(|x| x.start_ms)
        .and_then(|x| x.players.get(&m.me));
    for k in Accolade::ALL {
        let a = g
            .acts
            .get(&act)
            .and_then(|x| x.get(&k))
            .copied()
            .unwrap_or_default();
        let t = g
            .acts
            .get(ALL_TIME)
            .and_then(|x| x.get(&k))
            .copied()
            .unwrap_or_default();
        let fresh = latest.and_then(|e| e.get(&k)).is_some_and(|e| e.act_record);
        let best = |c: u32, b: f64| {
            if c == 0 {
                "—".to_string()
            } else {
                goldstars::format_value(k, b)
            }
        };
        rows.push(Row {
            segs: vec![
                s("  ", Style::new()),
                s(k.abbrev(), dim()),
                s(format!(" {:<12}", k.name()), text()),
                s(format!("{:>5}", format!("{}×", a.count)), bold()),
                s(" ", Style::new()),
                s(
                    format!("{:>4}", best(a.count, a.best)),
                    if fresh { gold() } else { text() },
                ),
                if fresh {
                    s(" new", gold())
                } else {
                    s("    ", Style::new())
                },
                s(format!("{:>5}", format!("{}×", t.count)), bold()),
                s(" ", Style::new()),
                s(format!("{:>4}", best(t.count, t.best)), text()),
            ],
            icons: vec![IconAt { column: 2, kind: k }],
            bg: None,
        });
    }
    rows
}

fn detail_screen(m: &Model, puuid: &str) -> Screen {
    let content = &m.content;
    let lobby = m
        .lobby
        .as_ref()
        .or(m.postgame.as_ref().and_then(|p| p.lobby.as_ref()));
    let slot = lobby.and_then(|l| l.slots.iter().find(|s| s.puuid == puuid));
    let incognito = slot.is_some_and(|s| s.incognito) && puuid != m.me;
    let d = if incognito {
        never_looked_up()
    } else {
        m.players.get(puuid).cloned().unwrap_or_default()
    };
    let agent = slot.and_then(|s| s.agent_id.clone());
    let title = if incognito {
        agent
            .as_deref()
            .map_or_else(|| "Player".into(), |a| content.agent(a))
    } else {
        d.name.as_ref().map_or_else(
            || {
                if puuid == m.me {
                    "you".into()
                } else {
                    LOADING.into()
                }
            },
            |RiotId { name, tag }| format!("{name}#{tag}"),
        )
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
    head.extend(partied_with(m, lobby, puuid));
    let map = m.lobby.as_ref().map(|l| content.map(&l.map_url));
    let mut left = act_rows(m, &d);
    let right = if puuid == m.me {
        left.push(blank());
        left.extend(map_rows(&d, map.as_deref()));
        accolade_rows(m)
    } else {
        map_rows(&d, map.as_deref())
    };
    let mut rows = vec![head.into(), blank()];
    let n = left.len().max(right.len());
    let (mut left, mut right) = (left.into_iter(), right.into_iter());
    for _ in 0..n {
        rows.push(two(
            left.next().unwrap_or_default(),
            right.next().unwrap_or_default(),
        ));
    }
    rows.push(blank());
    rows.extend(recent_rows(m, &d));
    Screen {
        title: vec![s(title, bold())],
        right: Vec::new(),
        rows,
        footer: keys(&[hint("esc", "back"), hint("?", "legend"), hint("q", "quit")]),
    }
}

/// What the details of an incognito player show: teamdiff never looks them up.
fn never_looked_up() -> PlayerData {
    PlayerData {
        profile: Load::Missing,
        maps: Load::Missing,
        recent: Load::Missing,
        ..Default::default()
    }
}

/// "partied with" and the names of the player's party, sorted, for the head of their details.
fn partied_with(m: &Model, lobby: Option<&Lobby>, puuid: &str) -> Vec<Seg> {
    let parties = m.parties();
    let Some(party) = parties.get(puuid) else {
        return Vec::new();
    };
    let mut mates: Vec<String> = parties
        .iter()
        .filter(|(p, id)| *id == party && *p != puuid)
        .map(|(p, _)| {
            let mate_slot = lobby.and_then(|l| l.slots.iter().find(|s| &s.puuid == p));
            name_segs(
                m,
                p,
                Visibility::of(mate_slot.is_some_and(|s| s.incognito)),
                mate_slot.and_then(|s| s.agent_id.as_deref()),
            )
            .iter()
            .map(|x| x.content.to_string())
            .collect()
        })
        .collect();
    mates.sort();
    if mates.is_empty() {
        return Vec::new();
    }
    vec![s("   partied with ", dim()), s(mates.join(", "), text())]
}

fn net_status(net: Option<&NetStats>) -> Vec<Seg> {
    let Some(net) = net else {
        return Vec::new();
    };
    let (started, finished) = (
        net.started.load(Ordering::Relaxed),
        net.finished.load(Ordering::Relaxed),
    );
    let limited = net
        .rate_limited_until
        .lock()
        .ok()
        .and_then(|g| *g)
        .filter(|t| *t > Instant::now());
    if started == finished && limited.is_none() {
        return Vec::new();
    }
    let t = match limited {
        Some(until) => format!(
            "loading {finished}/{started} · rate limited, retry {}s",
            until.saturating_duration_since(Instant::now()).as_secs() + 1
        ),
        None => format!("loading {finished}/{started}"),
    };
    vec![s("◐ ", warn()), s(t, warn())]
}

fn screen(m: &Model, ui: &UiState) -> Screen {
    match (&ui.view, &m.phase) {
        (View::Detail(p), _) => detail_screen(m, p),
        (_, Phase::Waiting(reason)) => waiting_screen(reason),
        (_, Phase::Menus) => menus_screen(m),
        (_, Phase::Pregame) => lobby_screen(m, ui, Stage::AgentSelect),
        (_, Phase::Ingame) => lobby_screen(m, ui, Stage::InGame),
        (_, Phase::Postgame) => postgame_screen(m, ui),
    }
}

fn too_small(f: &mut Frame<'_>, need_w: u16, need_h: u16) {
    let area = f.area();
    let msg = if area.width < need_w {
        format!("Widen to {need_w} columns (now {})", area.width)
    } else {
        format!("Make it taller: {need_h} rows needed (now {})", area.height)
    };
    let w = (cast::low_u16(msg.width()) + 6).max(24).min(area.width);
    let h = 6.min(area.height);
    let rect = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    let p = Paragraph::new(vec![
        Line::default(),
        Line::from(s("Terminal too small", bold())).centered(),
        Line::from(s(msg, dim())).centered(),
    ])
    .block(
        Block::bordered()
            .border_type(BorderType::Plain)
            .border_style(border()),
    );
    f.render_widget(p, rect);
}

const LEGEND_HEIGHT: u16 = cast::low_u16(Accolade::ICONS.len()) + 4;

fn legend(f: &mut Frame<'_>, origin: Rect, icons: &Icons) {
    let h = LEGEND_HEIGHT;
    let w = 30u16;
    let rect = Rect {
        x: origin.x + (origin.width.saturating_sub(w)) / 2,
        y: origin.y + (origin.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    let lines: Vec<Line<'_>> = iter::once(Line::default())
        .chain(Accolade::ICONS.iter().map(|k| {
            Line::from(vec![
                s("   ", Style::new()),
                s(k.abbrev(), dim()),
                s(format!("  {}", k.name()), text()),
            ])
        }))
        .collect();
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(border())
                .title(Line::from(vec![
                    s("─ ", border()),
                    s("ACCOLADES", bold()),
                    s(" ", border()),
                ]))
                .title_bottom(Line::from(keys(&[hint("esc", "close")]))),
        ),
        rect,
    );
    for (i, k) in Accolade::ICONS.iter().enumerate() {
        icons.draw(
            f,
            *k,
            Rect {
                x: rect.x + 4,
                y: rect.y + 2 + cast::low_u16(i),
                width: 2,
                height: 1,
            },
        );
    }
}

/// What the footer's right corner says: a recent request failure first, then the release notes
/// right after an update (above loading, which the first requests after starting would otherwise
/// show for most of its time), then loading, a ready update, and else the version.
fn status(m: &Model) -> Vec<Seg> {
    match &m.last_error {
        Some(LastError { message, at }) if at.elapsed() < ERROR_SHOWN_FOR => vec![
            s("⚠ ", bad()),
            s(message.chars().take(60).collect::<String>(), bad()),
        ],
        _ => match &m.updated {
            Some(Updated { version, started }) if started.elapsed() < UPDATED_SHOWN_FOR => {
                vec![s(format!("new in {version}: {RELEASES}"), good())]
            }
            _ => match (net_status(m.net.as_deref()), &m.update_ready) {
                (net, _) if !net.is_empty() => net,
                (_, Some(version)) => vec![s(
                    format!("teamdiff {version} is ready · restart to use it"),
                    good(),
                )],
                (_, None) => vec![s(concat!("v", env!("CARGO_PKG_VERSION")), dim())],
            },
        },
    }
}

/// Draws the screen for `m` and `ui`, or the too-small screen when the terminal can't fit it.
pub(crate) fn draw(f: &mut Frame<'_>, m: &Model, ui: &UiState, icons: &Icons) {
    let sc = screen(m, ui);
    let status = status(m);
    let need_h = cast::low_u16(sc.rows.len())
        .wrapping_add(2)
        .max(if ui.legend { LEGEND_HEIGHT } else { 0 });
    let area = f.area();
    if area.width < WIDTH || area.height < need_h {
        too_small(f, WIDTH, need_h);
        return;
    }
    let rect = Rect {
        x: area.x,
        y: area.y,
        width: WIDTH,
        height: need_h,
    };
    let mut block = Block::bordered()
        .border_style(border())
        .padding(Padding::horizontal(1))
        .title(Line::from(
            [vec![s("─ ", border())], sc.title, vec![s(" ", border())]].concat(),
        ));
    if !sc.right.is_empty() {
        block = block.title(
            Line::from([vec![s(" ", border())], sc.right, vec![s(" ─", border())]].concat())
                .right_aligned(),
        );
    }
    block = block.title_bottom(Line::from(
        [vec![s("─ ", border())], sc.footer, vec![s(" ", border())]].concat(),
    ));
    if !status.is_empty() {
        block = block.title_bottom(
            Line::from([vec![s(" ", border())], status, vec![s(" ─", border())]].concat())
                .right_aligned(),
        );
    }
    let inner = block.inner(rect);
    let lines: Vec<Line<'_>> = sc
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
    for (y, row) in sc.rows.iter().enumerate() {
        for icon in &row.icons {
            let r = Rect {
                x: inner.x + cast::low_u16(icon.column),
                y: inner.y.wrapping_add(cast::low_u16(y)),
                width: 2,
                height: 1,
            };
            if r.right() <= inner.right() && r.y < inner.bottom() {
                icons.draw(f, icon.kind, r);
            }
        }
    }
    if ui.legend {
        legend(f, rect, icons);
    }
}

/// True when the screen animates and needs frequent redraws.
pub(crate) fn animating(m: &Model) -> bool {
    m.phase == Phase::Postgame
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use teamdiff_core::demo;

    use super::*;
    use crate::rendering;

    fn footer(m: &Model) -> String {
        rendering::render(m, &UiState::default(), WIDTH, 10)
            .into_iter()
            .find(|l| l.starts_with('└'))
            .unwrap()
    }

    #[test]
    fn fit_pads_and_truncates_by_display_width() {
        assert_eq!(width(&fit(vec![s("abc", Style::new())], 6, Align::Left)), 6);
        let t = fit(
            vec![s("Charlie#DEMO-and-more", Style::new())],
            10,
            Align::Left,
        );
        assert_eq!(width(&t), 10);
        assert!(
            t.iter()
                .map(|x| x.content.to_string())
                .collect::<String>()
                .ends_with('…')
        );
        let wide = fit(vec![s("한국어이름", Style::new())], 6, Align::Left);
        assert_eq!(
            width(&wide),
            6,
            "double-width characters are measured by cells"
        );
    }

    #[test]
    fn map_header_falls_back_for_long_names() {
        assert_eq!(lobby_cols("Ascent")[8].title, "WIN% ON ASCENT");
        assert_eq!(lobby_cols("Corrode")[8].title, "CORRODE WIN%");
    }

    #[test]
    fn banner_reveals_left_to_right() {
        let at = |ms| {
            banner("VICTORY", Outcome::Win, ms)
                .iter()
                .map(|r| width(&r.segs))
                .max()
                .unwrap()
        };
        assert_eq!(at(0), at(10_000), "rows keep their width while revealing");
        let visible = |ms| {
            banner("VICTORY", Outcome::Win, ms)
                .iter()
                .flat_map(|r| r.segs.clone())
                .filter(|x| x.content == "█")
                .count()
        };
        assert!(visible(0) < visible(300));
        assert!(visible(300) < visible(5_000));
    }

    #[test]
    fn players_without_stats_say_why() {
        let mut m = demo::model("in-game");
        m.players.get_mut("neon").unwrap().profile = Load::Missing;
        let lines = rendering::render(&m, &UiState::default(), 120, 22);
        let row = |agent: &str| {
            lines
                .iter()
                .find(|l| l.contains(agent))
                .cloned()
                .unwrap_or_default()
        };
        assert!(row("Omen").contains("incognito"), "{}", row("Omen"));
        assert!(row("Neon").contains("not found"), "{}", row("Neon"));
    }

    #[test]
    fn request_errors_show_in_the_footer() {
        let mut m = Model::new(Content::default());
        let err = "500 Internal Server Error from /name-service/v2/players, with a reason long \
                   enough to be cut off"
            .to_string();
        m.last_error = Some(LastError {
            message: err.clone(),
            at: Instant::now(),
        });
        let lines = rendering::render(&m, &UiState::default(), WIDTH, 10);
        let footer = lines.iter().find(|l| l.starts_with('└')).unwrap();
        assert!(
            footer.contains("⚠ 500 Internal Server Error from /name-service/v2/players"),
            "the status survives truncation: {footer}"
        );
        m.last_error = Some(LastError {
            message: err,
            at: Instant::now().checked_sub(ERROR_SHOWN_FOR).unwrap(),
        });
        let lines = rendering::render(&m, &UiState::default(), WIDTH, 10);
        assert!(
            !lines.iter().any(|l| l.contains('⚠')),
            "old errors fade out"
        );
    }

    #[test]
    fn the_footer_shows_the_version_when_nothing_else_is_going_on() {
        let mut m = Model::new(Content::default());
        assert!(
            footer(&m).contains(concat!(" v", env!("CARGO_PKG_VERSION"), " ")),
            "{}",
            footer(&m)
        );
        m.last_error = Some(LastError {
            message: "503 Service Unavailable".into(),
            at: Instant::now(),
        });
        assert!(
            !footer(&m).contains(concat!(" v", env!("CARGO_PKG_VERSION"))),
            "an error takes its place"
        );
    }

    #[test]
    fn after_an_update_the_footer_links_the_release_notes_beside_the_longest_keys() {
        let mut m = demo::model("in-game");
        m.updated = Some(Updated {
            version: "0.12.0".parse().unwrap(),
            started: Instant::now(),
        });
        let lines = rendering::render(&m, &UiState::default(), WIDTH, 40);
        let footer = lines.iter().find(|l| l.starts_with('└')).unwrap();
        assert!(
            footer.contains("enter details  ? legend  q quit"),
            "{footer}"
        );
        assert!(
            footer.contains("new in 0.12.0: https://github.com/265866/teamdiff/releases "),
            "{footer}"
        );
    }

    #[test]
    fn the_release_notes_link_shows_over_loading_then_fades() {
        let mut m = Model::new(Content::default());
        let net = NetStats::default();
        net.started.store(3, Ordering::Relaxed);
        m.net = Some(Arc::new(net));
        m.update_ready = Some("0.13.0".parse().unwrap());
        m.updated = Some(Updated {
            version: "0.12.0".parse().unwrap(),
            started: Instant::now(),
        });
        assert!(
            footer(&m).contains("new in 0.12.0") && !footer(&m).contains("loading"),
            "{}",
            footer(&m)
        );
        m.updated = Some(Updated {
            version: "0.12.0".parse().unwrap(),
            started: Instant::now().checked_sub(UPDATED_SHOWN_FOR).unwrap(),
        });
        assert!(
            footer(&m).contains("loading 0/3") && !footer(&m).contains("new in"),
            "{}",
            footer(&m)
        );
        m.net.as_ref().unwrap().finished.store(3, Ordering::Relaxed);
        assert!(
            footer(&m).contains("teamdiff 0.13.0 is ready · restart to use it"),
            "a ready update stays: {}",
            footer(&m)
        );
        m.update_ready = None;
        assert!(
            footer(&m).contains(concat!(" v", env!("CARGO_PKG_VERSION"), " ")),
            "{}",
            footer(&m)
        );
    }

    #[test]
    fn a_ready_update_shows_in_the_footer_when_nothing_else_does() {
        let mut m = Model::new(Content::default());
        m.update_ready = Some("0.3.0".parse().unwrap());
        let lines = rendering::render(&m, &UiState::default(), WIDTH, 10);
        let footer = lines.iter().find(|l| l.starts_with('└')).unwrap();
        assert!(
            footer.contains("teamdiff 0.3.0 is ready · restart to use it"),
            "{footer}"
        );
        m.last_error = Some(LastError {
            message: "503 Service Unavailable".into(),
            at: Instant::now(),
        });
        let lines = rendering::render(&m, &UiState::default(), WIDTH, 10);
        let footer = lines.iter().find(|l| l.starts_with('└')).unwrap();
        assert!(
            footer.contains("⚠ 503") && !footer.contains("restart"),
            "an error shows first: {footer}"
        );
    }

    #[test]
    fn too_small_screen_reports_width() {
        let m = Model::new(Content::default());
        let lines = rendering::render(&m, &UiState::default(), 80, 20);
        assert!(lines.iter().any(|l| l.contains("Terminal too small")));
        assert!(
            lines
                .iter()
                .any(|l| l.contains("Widen to 112 columns (now 80)"))
        );
    }

    #[test]
    fn riots_flags_decide_the_outcome() {
        let m = demo::model("defeat");
        let mut s = (**m.postgame.as_ref().unwrap().summary.as_ref().unwrap()).clone();
        let mine = s.player(&m.me).unwrap().team.clone();
        for result in s.teams.values_mut() {
            result.rounds_won = 6;
        }
        assert_eq!(
            outcome(&s, &mine),
            Outcome::Loss,
            "a surrender at a tied score"
        );
        for result in s.teams.values_mut() {
            result.won = false;
        }
        assert_eq!(outcome(&s, &mine), Outcome::Draw, "no winner");
        s.teams.get_mut(&mine).unwrap().won = true;
        assert_eq!(outcome(&s, &mine), Outcome::Win);
    }
}
