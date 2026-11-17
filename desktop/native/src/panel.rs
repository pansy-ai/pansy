//! The quick panel's place under Hyprland (Omarchy's compositor), as
//! magpie's. A Wayland window can't place itself, so the panel, about to
//! open, is given a window rule of its own: floating and pinned to every
//! workspace, under the bar with its middle at the pointer (which has just
//! clicked the tray icon). And as Hyprland tells a program nothing of clicks
//! on other windows, while Omarchy's focus follows the mouse (the pointer
//! merely passing over another window takes it, so losing it says nothing),
//! a left-click bind that lets the click through announces each one on
//! Hyprland's event socket while the panel is open.
//!
//! Hyprland keeps a floating window's middle where it was when the window
//! changes its own size, so the panel, growing to a taller page, would climb
//! over the bar and off the screen: a timer in Hyprland holds its corner in
//! place while it is open.
//!
//! The bind is never removed, only switched off: Hyprland removes every bind
//! on the key at once (magpie's own left-click bind too), and calling on a
//! bind removed that way brings Hyprland down. One found gone is made again.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;

/// How far a drop-down panel keeps from the bar and the screen's edge, as
/// Omarchy's own popups do.
const GAP: i32 = 6;

/// The event the click bind announces, and the rule's name.
const CLICK: &str = "pansy-click";
const RULE: &str = "pansy-panel";

/// How often, in milliseconds, Hyprland puts the open panel back in place.
const HOLD_MS: u32 = 8;

/// Counts the panel's openings and closings: a wait for a click outside it
/// gives up once the panel it waited on has gone.
static TURN: AtomicU64 = AtomicU64::new(0);

/// Whether pansy runs under Hyprland.
pub fn hyprland() -> bool {
    cfg!(target_os = "linux")
        && std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|s| !s.is_empty())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct Monitor {
    name: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    scale: f64,
    transform: i32,
    /// Left, top, right, bottom: the bar's room.
    reserved: [i32; 4],
}

impl Default for Monitor {
    fn default() -> Self {
        Monitor {
            name: String::new(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            scale: 1.0,
            transform: 0,
            reserved: [0; 4],
        }
    }
}

impl Monitor {
    /// Its size in layout coordinates, the ones windows and the pointer are
    /// placed in.
    fn size(&self) -> (i32, i32) {
        let s = if self.scale > 0.0 { self.scale } else { 1.0 };
        let (w, h) = (
            (f64::from(self.width) / s).round() as i32,
            (f64::from(self.height) / s).round() as i32,
        );
        if self.transform % 2 == 1 {
            (h, w)
        } else {
            (w, h)
        }
    }
}

/// Where the panel goes: on `monitor`, at `x`, `y` from its top left corner
/// (as Hyprland's move rule counts), with `room` down to the bottom of its
/// work area; `corner` is the monitor's own, where those count from.
#[derive(Debug, PartialEq)]
struct Place {
    monitor: String,
    x: i32,
    y: i32,
    room: i32,
    corner: (i32, i32),
}

/// Where a panel `width` wide goes for the pointer at `cx`, `cy` (layout
/// coordinates across every monitor): under the bar, its middle at the
/// pointer, kept `GAP` inside the monitor the pointer is on.
fn place_at(width: i32, cx: i32, cy: i32, monitors: &[Monitor]) -> Option<Place> {
    monitors.iter().find_map(|m| {
        let (w, h) = m.size();
        if cx < m.x || cx >= m.x + w || cy < m.y || cy >= m.y + h {
            return None;
        }
        let (left, right) = (m.reserved[0] + GAP, w - m.reserved[2] - GAP);
        let x = (cx - m.x - width / 2).min(right - width).max(left);
        let y = m.reserved[1] + GAP;
        Some(Place {
            monitor: m.name.clone(),
            x,
            y,
            room: h - m.reserved[3] - GAP - y,
            corner: (m.x, m.y),
        })
    })
}

/// The panel's rule: floating, pinned to every workspace, on the monitor
/// clicked at its place there, without the border, the shadow or the
/// opening animation, and opaque (Omarchy dims its other windows by a tag).
fn rule(title: &str, at: &Place) -> String {
    let on = if at.monitor.is_empty() {
        String::new()
    } else {
        format!("monitor = {}, ", lua_string(&at.monitor))
    };
    format!(
        "hl.window_rule({{ name = {}, match = {{ title = {} }}, tag = \"-default-opacity\", float = true, pin = true, border_size = 0, no_shadow = true, opacity = \"1 1\", no_anim = true, {on}move = {{ {}, {} }} }})",
        lua_string(RULE),
        lua_string(&format!("^{}$", regex_quote(title))),
        at.x,
        at.y,
    )
}

/// Has Hyprland announce left clicks, and hold the panel titled `title` at
/// `at` (across every monitor), until `RELEASE`: one bind and one timer,
/// made once and switched on and off, kept in globals a config reload
/// clears.
fn hold(title: &str, at: &Place) -> String {
    format!(
        "pansy_panel = {{ title = {}, x = {}, y = {} }} \
         local function click() \
           if pansy_click == nil or tostring(pansy_click) == \"HL.Keybind(expired)\" then \
             pansy_click = hl.bind(\"mouse:272\", function() hl.dispatch(hl.dsp.event({})) end, {{ non_consuming = true }}) \
           else pansy_click:set_enabled(true) end \
         end \
         click() \
         if pansy_hold == nil or tostring(pansy_hold) == \"HL.Timer(expired)\" then \
           pansy_hold = hl.timer(function() \
             local p = pansy_panel \
             if p == nil then return end \
             click() \
             for _, w in ipairs(hl.get_windows()) do \
               if w.title == p.title and w.floating then \
                 local now = w.at \
                 if now.x ~= p.x or now.y ~= p.y then hl.dispatch(hl.dsp.window.move({{ x = p.x, y = p.y, window = w }})) end \
               end \
             end \
           end, {{ type = \"repeat\", timeout = {HOLD_MS} }}) \
         else pansy_hold:set_enabled(true) end",
        lua_string(title),
        at.corner.0 + at.x,
        at.corner.1 + at.y,
        lua_string(CLICK),
    )
}

/// Lets the panel go: no clicks announced, nothing held.
const RELEASE: &str = "pansy_panel = nil \
     if pansy_hold ~= nil and tostring(pansy_hold) ~= \"HL.Timer(expired)\" then pansy_hold:set_enabled(false) end \
     if pansy_click ~= nil and tostring(pansy_click) ~= \"HL.Keybind(expired)\" then pansy_click:set_enabled(false) end";

/// Whether a click at `x`, `y` is off the panel at `at`, `size`, and below
/// the bar: a click on the bar is left to the icon it lands on, so the
/// tray icon closes the panel rather than closing and opening it again.
fn outside(x: i32, y: i32, at: [i32; 2], size: [i32; 2]) -> bool {
    if y < at[1] - GAP {
        return false;
    }
    x < at[0] || x >= at[0] + size[0] || y < at[1] || y >= at[1] + size[1]
}

fn regex_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if !(c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn lua_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned())
}

fn hyprctl_json<T: for<'de> Deserialize<'de>>(what: &str) -> anyhow::Result<T> {
    let out = Command::new("hyprctl").args(["-j", what]).output()?;
    if !out.status.success() {
        anyhow::bail!(
            "hyprctl {what}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(serde_json::from_slice(&out.stdout)?)
}

fn hyprctl_eval(lua: &str) -> anyhow::Result<()> {
    let out = Command::new("hyprctl").args(["eval", lua]).output()?;
    let said = String::from_utf8_lossy(&out.stdout);
    let said = said.trim();
    if !out.status.success() || !(said.is_empty() || said == "ok") {
        anyhow::bail!("hyprctl eval: {said}");
    }
    Ok(())
}

#[derive(Deserialize)]
struct Pointer {
    x: i32,
    y: i32,
}

/// Readies the panel titled `title`, `width` points wide, to open under the
/// bar at the pointer, and has Hyprland announce clicks until it closes.
/// The room it has down to the screen's foot; 0 where the system places it.
pub fn place(title: &str, width: i32) -> anyhow::Result<i32> {
    TURN.fetch_add(1, Ordering::SeqCst);
    if !hyprland() {
        return Ok(0);
    }
    let pointer: Pointer = hyprctl_json("cursorpos")?;
    let monitors: Vec<Monitor> = hyprctl_json("monitors")?;
    let Some(at) = place_at(width, pointer.x, pointer.y, &monitors) else {
        return Ok(0);
    };
    hyprctl_eval(&rule(title, &at))?;
    hyprctl_eval(&hold(title, &at))?;
    Ok(at.room.max(0))
}

/// The panel closed: Hyprland stops announcing clicks and holding it, and a
/// wait for a click gives up.
pub fn closed() -> anyhow::Result<()> {
    TURN.fetch_add(1, Ordering::SeqCst);
    if hyprland() {
        hyprctl_eval(RELEASE)?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct Client {
    #[serde(default)]
    title: String,
    #[serde(default)]
    at: [i32; 2],
    #[serde(default)]
    size: [i32; 2],
}

/// Whether the pointer, which has just clicked, is off the window titled
/// `title` and below the bar.
fn clicked_outside(title: &str) -> bool {
    let (Ok(pointer), Ok(clients)) = (
        hyprctl_json::<Pointer>("cursorpos"),
        hyprctl_json::<Vec<Client>>("clients"),
    ) else {
        return false;
    };
    clients
        .iter()
        .find(|c| c.title == title)
        .is_some_and(|c| outside(pointer.x, pointer.y, c.at, c.size))
}

/// Waits for a click outside the panel titled `title`: `true` at the first,
/// `false` once the panel has gone (or off Hyprland).
#[cfg(unix)]
pub fn wait_outside(title: &str) -> bool {
    use std::io::{BufRead, BufReader, ErrorKind};
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let turn = TURN.load(Ordering::SeqCst);
    if !hyprland() {
        return false;
    }
    let (Some(runtime), Some(instance)) = (
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE"),
    ) else {
        return false;
    };
    let socket = std::path::Path::new(&runtime)
        .join("hypr")
        .join(instance)
        .join(".socket2.sock");
    let Ok(stream) = UnixStream::connect(socket) else {
        return false;
    };
    if stream
        .set_read_timeout(Some(Duration::from_millis(400)))
        .is_err()
    {
        return false;
    }
    let mut events = BufReader::new(stream);
    let mut line = String::new();
    let announced = format!("custom>>{CLICK}");
    while TURN.load(Ordering::SeqCst) == turn {
        match events.read_line(&mut line) {
            Ok(0) => return false,
            Ok(_) => {
                if line.trim_end() == announced && clicked_outside(title) {
                    return TURN.load(Ordering::SeqCst) == turn;
                }
                line.clear();
            }
            // what came before the pause stays in `line` for the rest of it
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(_) => return false,
        }
    }
    false
}

#[cfg(not(unix))]
pub fn wait_outside(_title: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, x: i32, width: i32, scale: f64) -> Monitor {
        Monitor {
            name: name.to_owned(),
            x,
            width,
            height: 2160,
            scale,
            reserved: [0, 26, 0, 0],
            ..Monitor::default()
        }
    }

    #[test]
    fn opens_under_the_bar_at_the_pointer() {
        let mons = [monitor("HDMI-A-2", 0, 3840, 1.5)];
        let at = place_at(440, 1956, 10, &mons).unwrap();
        assert_eq!(
            at,
            Place {
                monitor: "HDMI-A-2".into(),
                x: 1736,
                y: 32,
                room: 1440 - 32 - GAP,
                corner: (0, 0),
            }
        );
    }

    #[test]
    fn keeps_inside_the_screen_near_its_edges() {
        let mons = [monitor("DP-1", 0, 1920, 1.0)];
        assert_eq!(place_at(440, 1910, 10, &mons).unwrap().x, 1920 - GAP - 440);
        assert_eq!(place_at(440, 5, 10, &mons).unwrap().x, GAP);
    }

    #[test]
    fn counts_from_the_corner_of_the_monitor_clicked() {
        let mons = [
            monitor("DP-1", 0, 1920, 1.0),
            monitor("DP-2", 1920, 1920, 1.0),
        ];
        let at = place_at(440, 1920 + 960, 10, &mons).unwrap();
        assert_eq!((at.monitor.as_str(), at.x), ("DP-2", 960 - 220));
        assert_eq!(at.corner, (1920, 0));
        assert!(place_at(440, 4000, 10, &mons).is_none());
    }

    #[test]
    fn a_click_on_the_bar_is_the_icons() {
        let (at, size) = ([100, 32], [440, 300]);
        assert!(!outside(200, 10, at, size));
        assert!(!outside(200, 100, at, size));
        assert!(outside(50, 100, at, size));
        assert!(outside(200, 400, at, size));
    }

    #[test]
    fn the_rule_names_the_panel_and_its_place() {
        let at = Place {
            monitor: "HDMI-A-2".into(),
            x: 1736,
            y: 32,
            room: 1000,
            corner: (0, 0),
        };
        let lua = rule("pansy panel", &at);
        assert!(
            lua.contains("match = { title = \"^pansy panel$\" }"),
            "{lua}"
        );
        assert!(
            lua.contains("monitor = \"HDMI-A-2\", move = { 1736, 32 }"),
            "{lua}"
        );
        assert_eq!(regex_quote("a.b (c)"), "a\\.b \\(c\\)");
    }

    #[test]
    fn holds_the_panel_across_monitors_and_never_removes_the_bind() {
        let at = Place {
            monitor: "DP-2".into(),
            x: 740,
            y: 32,
            room: 1000,
            corner: (1920, 0),
        };
        let lua = hold("pansy panel", &at);
        assert!(
            lua.contains("pansy_panel = { title = \"pansy panel\", x = 2660, y = 32 }"),
            "{lua}"
        );
        assert!(lua.contains("hl.dsp.event(\"pansy-click\")"), "{lua}");
        for said in [lua.as_str(), RELEASE] {
            assert!(!said.contains(":remove("), "{said}");
        }
        assert!(RELEASE.contains("pansy_hold:set_enabled(false)"));
    }
}
