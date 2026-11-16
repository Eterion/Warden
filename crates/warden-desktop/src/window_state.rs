//! The window's place: where it was last, its size, and whether it was maximized, kept in the data folder
//! (`window.json`) and given back when the window opens again, as standard Windows apps do.
//!
//! Sizes and positions are in physical pixels: the window's outer position and its inner size (the window has no
//! frame of its own, so the two line up).

use serde::{Deserialize, Serialize};

pub const FILE: &str = "window.json";

/// How much of the window's top strip (where it is dragged from) must be on a screen for a saved place to be used.
const GRAB_WIDTH: i64 = 96;
const GRAB_HEIGHT: i64 = 32;

/// Where the window was when it was last shown normally (not minimized), and whether it was maximized. The bounds are
/// the window's normal ones even while it is maximized, so un-maximizing goes back to them; `maximized_at` is where the
/// maximized window itself was, so it opens maximized on that screen even after moving there while maximized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximized_at: Option<[i32; 2]>,
}

/// A screen, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Screen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// What the window looks like after it moved or changed size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seen {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub minimized: bool,
    pub maximized: bool,
}

/// The place after the window moved or changed size. A minimized window keeps the place it had; a maximized one
/// keeps the bounds from before it was maximized.
pub fn follow(before: Option<Place>, seen: Seen) -> Option<Place> {
    if seen.minimized || seen.width == 0 || seen.height == 0 {
        return before;
    }
    if seen.maximized {
        // The normal bounds stay as they were; a window opened maximized with none known keeps what it shows.
        let at = Some([seen.x, seen.y]);
        return Some(match before {
            Some(p) => Place {
                maximized: true,
                maximized_at: at,
                ..p
            },
            None => Place {
                x: seen.x,
                y: seen.y,
                width: seen.width,
                height: seen.height,
                maximized: true,
                maximized_at: at,
            },
        });
    }
    Some(Place {
        x: seen.x,
        y: seen.y,
        width: seen.width,
        height: seen.height,
        maximized: false,
        maximized_at: None,
    })
}

/// The screen holding a point a little inside the window's corner (a maximized window's corner can sit just off its
/// screen).
fn screen_at(screens: &[Screen], x: i32, y: i32) -> Option<Screen> {
    let (px, py) = (i64::from(x) + 16, i64::from(y) + 16);
    screens.iter().copied().find(|s| {
        let (sl, st) = (i64::from(s.x), i64::from(s.y));
        px >= sl && px < sl + i64::from(s.width) && py >= st && py < st + i64::from(s.height)
    })
}

/// Where to put the window before showing it: a window maximized on another screen than its normal bounds (moved
/// there while maximized) gets its normal bounds moved onto that screen, so it maximizes there and un-maximizes there.
pub fn opening(place: Place, screens: &[Screen]) -> Place {
    let Some([mx, my]) = place.maximized_at.filter(|_| place.maximized) else {
        return place;
    };
    match (
        screen_at(screens, place.x, place.y),
        screen_at(screens, mx, my),
    ) {
        (Some(from), Some(to)) if from != to => Place {
            x: place.x.saturating_add(to.x.saturating_sub(from.x)),
            y: place.y.saturating_add(to.y.saturating_sub(from.y)),
            ..place
        },
        _ => place,
    }
}

/// The saved place, when it can be used: its top strip is on a screen far enough to grab, and it has a size. `None`
/// means the window opens at its default size, centred.
pub fn usable(saved: Option<Place>, screens: &[Screen]) -> Option<Place> {
    let place = saved.filter(|p| p.width > 0 && p.height > 0)?;
    let (left, top) = (i64::from(place.x), i64::from(place.y));
    let (right, bottom) = (
        left + i64::from(place.width),
        top + i64::from(place.height).min(GRAB_HEIGHT),
    );
    screens
        .iter()
        .any(|s| {
            let (sl, st) = (i64::from(s.x), i64::from(s.y));
            let (sr, sb) = (sl + i64::from(s.width), st + i64::from(s.height));
            let wide = right.min(sr) - left.max(sl);
            let high = bottom.min(sb) - top.max(st);
            wide >= GRAB_WIDTH && high >= GRAB_HEIGHT.min(i64::from(place.height))
        })
        .then_some(place)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Screen = Screen {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    fn place(x: i32, y: i32, maximized: bool) -> Place {
        Place {
            x,
            y,
            width: 1280,
            height: 800,
            maximized,
            maximized_at: None,
        }
    }

    fn seen(x: i32, y: i32, width: u32, height: u32) -> Seen {
        Seen {
            x,
            y,
            width,
            height,
            minimized: false,
            maximized: false,
        }
    }

    #[test]
    fn a_place_round_trips_through_its_file() {
        let p = place(-1500, 40, true);
        let json = serde_json::to_string(&p).expect("json");
        assert_eq!(
            json,
            r#"{"x":-1500,"y":40,"width":1280,"height":800,"maximized":true}"#
        );
        assert_eq!(serde_json::from_str::<Place>(&json).expect("place"), p);
        let on = Place {
            maximized_at: Some([-1928, -8]),
            ..p
        };
        let json = serde_json::to_string(&on).expect("json");
        assert!(
            json.ends_with(r#""maximized":true,"maximizedAt":[-1928,-8]}"#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Place>(&json).expect("place"), on);
    }

    #[test]
    fn a_window_maximized_onto_another_screen_opens_there() {
        let left = Screen {
            x: -2560,
            y: 0,
            width: 2560,
            height: 1440,
        };
        let screens = [SCREEN, left];
        // Normal on the main screen, then maximized there and moved to the left screen while maximized.
        let normal = Some(place(100, 50, false));
        let here = follow(
            normal,
            Seen {
                maximized: true,
                ..seen(-8, -8, 1936, 1048)
            },
        );
        let moved = follow(
            here,
            Seen {
                maximized: true,
                ..seen(-2568, -8, 2576, 1408)
            },
        )
        .expect("place");
        assert_eq!(moved.maximized_at, Some([-2568, -8]));
        assert_eq!((moved.x, moved.y), (100, 50), "the normal bounds stay");
        // It opens with its normal bounds moved onto the left screen, so it maximizes there.
        let open = opening(moved, &screens);
        assert_eq!((open.x, open.y, open.width), (-2460, 50, 1280));
        // Maximized on the screen it was normal on, or that screen gone: as saved.
        assert_eq!(
            opening(here.expect("place"), &screens),
            here.expect("place")
        );
        assert_eq!(opening(moved, &[SCREEN]), moved);
        // Un-maximized: where it was maximized no longer matters.
        assert_eq!(
            follow(Some(moved), seen(100, 50, 1280, 800)).and_then(|p| p.maximized_at),
            None
        );
    }

    #[test]
    fn moving_and_resizing_is_followed() {
        let p = follow(None, seen(100, 50, 1000, 700));
        assert_eq!(
            p,
            Some(Place {
                x: 100,
                y: 50,
                width: 1000,
                height: 700,
                maximized: false,
                maximized_at: None,
            })
        );
        assert_eq!(follow(p, seen(300, 60, 1000, 700)).map(|p| p.x), Some(300));
    }

    #[test]
    fn maximizing_keeps_the_size_from_before_and_minimizing_keeps_everything() {
        let normal = Some(place(100, 50, false));
        let maximized = follow(
            normal,
            Seen {
                maximized: true,
                ..seen(-8, -8, 1936, 1048)
            },
        );
        assert_eq!(
            maximized,
            Some(Place {
                maximized_at: Some([-8, -8]),
                ..place(100, 50, true)
            })
        );
        // Minimized: Windows reports the window far off screen with no size; nothing changes.
        let minimized = Seen {
            minimized: true,
            ..seen(-32000, -32000, 0, 0)
        };
        assert_eq!(follow(maximized, minimized), maximized);
        assert_eq!(follow(normal, seen(-32000, -32000, 0, 0)), normal);
        // Restored from maximized: the normal bounds again.
        assert_eq!(
            follow(maximized, seen(100, 50, 1280, 800)),
            Some(place(100, 50, false))
        );
        // Opened maximized with nothing saved: what it shows, maximized.
        assert_eq!(
            follow(
                None,
                Seen {
                    maximized: true,
                    ..seen(0, 0, 1920, 1040)
                }
            )
            .map(|p| p.maximized),
            Some(true)
        );
    }

    #[test]
    fn a_place_off_every_screen_is_not_used() {
        assert_eq!(usable(None, &[SCREEN]), None);
        assert_eq!(
            usable(Some(place(100, 50, false)), &[SCREEN]),
            Some(place(100, 50, false))
        );
        // On a second screen to the left that is still there, and gone.
        let left = Screen {
            x: -2560,
            y: 0,
            width: 2560,
            height: 1440,
        };
        let there = place(-2000, 100, false);
        assert_eq!(usable(Some(there), &[SCREEN, left]), Some(there));
        assert_eq!(usable(Some(there), &[SCREEN]), None);
        // Its top strip just off the bottom, or only a sliver at the edge to grab: not used.
        assert_eq!(usable(Some(place(100, 1070, false)), &[SCREEN]), None);
        assert_eq!(usable(Some(place(1900, 100, false)), &[SCREEN]), None);
        // Enough of the top strip to grab at the right edge: used.
        assert!(usable(Some(place(1800, 100, false)), &[SCREEN]).is_some());
        // No size, or no screens known: not used.
        let flat = Place {
            width: 0,
            ..place(100, 50, false)
        };
        assert_eq!(usable(Some(flat), &[SCREEN]), None);
        assert_eq!(usable(Some(place(100, 50, false)), &[]), None);
    }
}
