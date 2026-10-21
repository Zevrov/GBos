//! EXPAND's camera: ZOOM and the look-around, over the overworld the Android app's pk_wide.c draws past the GBA's
//! edges. Which window of the scene the game view shows each frame. A port of WideCamera.kt - keep in step.
//!
//! - **Zoom steps**: 1x is the game as large as the view shows all of it (its full height on a wide screen); each
//!   step out is a whole number of screen pixels per GBA pixel, so the pixel art stays crisp, and shows at least
//!   [`MIN_STEP`] more than the one before, inside pk_wide.h's largest window.
//! - **Close up when it matters**: the game wanting it (a battle starting, a menu screen, a text box) pulls in to 1x
//!   - one timed, eased motion per event - and the chosen step comes back, as gently, once that's been over a moment.
//! - **Look-around**: standing, it moves the view, which stays where it's left; walking glides it back to the player
//!   - or, still held, follows it but always keeps the player on screen.

/// What the last frame was (pk_wide.h's PK_WIDE_SCENE_*).
pub const SCENE_FIELD: i32 = 0;
pub const SCENE_TEXT: i32 = 1;
pub const SCENE_OVERLAY: i32 = 2;
pub const SCENE_BATTLE_START: i32 = 3;
#[allow(dead_code)] // what wide_scene() says with EXPAND off; the camera treats it as any other close-up
pub const SCENE_OFF: i32 = 4;
pub const SCENE_FADE: i32 = 5;
pub const SCENE_MENU: i32 = 6;

/// The game frame, and pk_wide.h's largest window (PK_WIDE_MAX_VIEW_W / _H).
pub const PK_W: f32 = 240.0;
pub const PK_H: f32 = 160.0;
pub const MAX_W: i32 = 800;
pub const MAX_H: i32 = 800;

/// A step out must show at least this much more of the map than the one before it.
const MIN_STEP: f32 = 1.2;
/// The pull-in to 1x: a text box or a menu in under half a second; a battle over its first flashes. The way back
/// out, slower.
const PULL_IN: f32 = 0.42;
const PULL_IN_BATTLE: f32 = 0.55;
const ZOOM_OUT: f32 = 0.65;
/// s: the view following the look-around / going back for a close-up; gliding back to a player who walks.
const LOOK_TAU: f32 = 0.12;
const RETURN_TAU: f32 = 0.18;
/// GBA px per s at full look, at 1x.
const LOOK_SPEED: f32 = 150.0;
/// GBA px: the player's sprite stays at least this far inside the view.
const PLAYER_KEEP: f32 = 28.0;
/// s: the close view is over for this long before the zoom goes back out.
const RETURN_DELAY: f32 = 0.5;

/// What the screen shows: the window pk_wide drew (`x`, `y`, `w`, `h` in the frame's GBA pixels), placed on the
/// view at (`off_x`, `off_y`) screen pixels from its top-left (<= 0: the window's whole pixels reach past the view
/// by the fraction the look-around / zoom has moved), `scale` screen pixels per GBA pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub off_x: f32,
    pub off_y: f32,
    pub scale: f32,
}

impl Window {
    /// Where the area-name banner goes in the window (its pixels): its top-left as much of it as the view shows,
    /// below the `top_clear` screen pixels the bar of buttons covers. The game draws it at the frame's corner, which
    /// EXPAND puts towards the middle of the view.
    pub fn banner_at(&self, top_clear: f32) -> (i32, i32) {
        let x = (-self.off_x / self.scale).ceil().max(0.0);
        let y = ((top_clear - self.off_y) / self.scale).ceil().max(0.0);
        (x as i32, y as i32)
    }
}

#[derive(Default)]
pub struct Camera {
    /// The zoom step picked: 0 = 1x.
    pub level: usize,
    /// The look-around, -1..1 per axis (right / down positive).
    pub look_x: f32,
    pub look_y: f32,
    /// A direction is held: the player is walking (or trying to).
    pub walking: bool,
    /// SETTINGS: a text box or a menu screen pulls in to 1x too (battles and anything off the map always do).
    pub zoom_on_text: bool,

    scale: f32,
    pan_x: f32,
    pan_y: f32,
    /// Seconds of the camera's own clock (frames' dt added up).
    clock: f32,
    /// When the game last stopped wanting a close view (None: it still does).
    close_over_since: Option<f32>,
    tween_from: f32,
    tween_target: f32,
    tween_start: f32,
    tween_len: f32,
}

/// Screen pixels per GBA pixel for each step on a `w` x `h` view: 1x (the whole frame as large as it fits), then the
/// whole numbers below it that show at least [`MIN_STEP`] more each and fit pk_wide's largest window.
pub fn levels_for(w: f32, h: f32) -> Vec<f32> {
    if w <= 0.0 || h <= 0.0 {
        return vec![1.0];
    }
    let fit = (w / PK_W).min(h / PK_H);
    let mut out = vec![fit];
    let mut k = fit.floor() as i32;
    if k as f32 == fit {
        k -= 1;
    }
    while k >= 1 {
        if w / k as f32 > MAX_W as f32 || h / k as f32 > MAX_H as f32 {
            break;
        }
        if out.last().unwrap() / k as f32 >= MIN_STEP {
            out.push(k as f32);
        }
        k -= 1;
    }
    out
}

fn smoothstep(p: f32) -> f32 {
    p * p * (3.0 - 2.0 * p)
}

fn ease(dt: f32, tau: f32) -> f32 {
    1.0 - (-dt / tau).exp()
}

impl Camera {
    pub fn new() -> Camera {
        Camera {
            zoom_on_text: true,
            ..Default::default()
        }
    }

    /// The next step out, back to 1x after the last, on a `w` x `h` view; returns how much more of the map it shows.
    pub fn cycle(&mut self, w: f32, h: f32) -> f32 {
        let l = levels_for(w, h);
        self.level = if l.len() <= 1 {
            0
        } else {
            (self.level + 1) % l.len()
        };
        l[0] / l[self.level.min(l.len() - 1)]
    }

    /// Back to 1x on the player at once (a new game, a state load, EXPAND switched on).
    pub fn reset(&mut self) {
        self.scale = 0.0;
        self.pan_x = 0.0;
        self.pan_y = 0.0;
        self.tween_target = 0.0;
    }

    fn spring(&mut self, tx: f32, ty: f32, dt: f32, tau: f32) {
        let k = ease(dt, tau);
        self.pan_x += (tx - self.pan_x) * k;
        self.pan_y += (ty - self.pan_y) * k;
        if (self.pan_x - tx).hypot(self.pan_y - ty) < 0.05 {
            self.pan_x = tx;
            self.pan_y = ty;
        }
    }

    /// Before a frame: the window to draw on a `vw` x `vh` view (screen pixels), `dt` seconds after the last, given
    /// what the last frame was (`scene`). None without a view yet.
    pub fn frame(&mut self, vw: f32, vh: f32, scene: i32, dt: f32) -> Option<Window> {
        if vw <= 0.0 || vh <= 0.0 {
            return None;
        }
        let steps = levels_for(vw, vh);
        let fit = steps[0];
        let dt = dt.clamp(0.0, 0.1);
        self.clock += dt;
        let now = self.clock;

        let wants_close = match scene {
            SCENE_FIELD => false,
            SCENE_TEXT | SCENE_MENU => self.zoom_on_text,
            _ => true,
        };
        // A fade decides nothing; nor does a picture over the field (FireRed's map preview).
        if scene == SCENE_FADE || scene == SCENE_OVERLAY {
        } else if wants_close {
            self.close_over_since = None;
        } else if self.close_over_since.is_none() {
            self.close_over_since = Some(now);
        }
        let close = self.close_over_since.is_none_or(|t| now - t < RETURN_DELAY);
        let target = if close {
            fit
        } else {
            steps[self.level.min(steps.len() - 1)]
        };
        // A timed ease-in-out in log space (the same feel at every step): a new target starts a new motion from
        // wherever the last one is.
        if target != self.tween_target {
            self.tween_from = if self.scale <= 0.0 {
                target
            } else {
                self.scale
            };
            self.tween_target = target;
            self.tween_start = now;
            self.tween_len = if self.scale <= 0.0 {
                0.0
            } else if target > self.tween_from && scene == SCENE_BATTLE_START {
                PULL_IN_BATTLE
            } else if target > self.tween_from {
                PULL_IN
            } else {
                ZOOM_OUT
            };
        }
        let p = if self.tween_len <= 0.0 {
            1.0
        } else {
            ((now - self.tween_start) / self.tween_len).clamp(0.0, 1.0)
        };
        self.scale = self.tween_from * (self.tween_target / self.tween_from).powf(smoothstep(p));
        if (self.scale - target).abs() < 0.002 {
            self.scale = target;
        }

        let fw = vw / self.scale;
        let fh = vh / self.scale;
        let (sx, sy) = (self.look_x.clamp(-1.0, 1.0), self.look_y.clamp(-1.0, 1.0));
        let looking = sx != 0.0 || sy != 0.0;
        if close {
            self.spring(0.0, 0.0, dt, LOOK_TAU);
        } else if self.walking && looking {
            // Follow the look, the player kept on screen: the view's centre at most this far off them.
            let max_x = (fw / 2.0 - PLAYER_KEEP).max(0.0);
            let max_y = (fh / 2.0 - PLAYER_KEEP).max(0.0);
            self.spring(sx * max_x, sy * max_y, dt, LOOK_TAU);
        } else if self.walking {
            self.spring(0.0, 0.0, dt, RETURN_TAU);
        } else if looking {
            let speed = LOOK_SPEED * (fit / self.scale).max(1.0); // further out, the view travels faster
            self.pan_x += sx * speed * dt;
            self.pan_y += sy * speed * dt;
        }
        // As far as a screen's worth of map past the game's own view each way.
        let (lim_x, lim_y) = (fw.max(PK_W), fh.max(PK_H));
        self.pan_x = self.pan_x.clamp(-lim_x, lim_x);
        self.pan_y = self.pan_y.clamp(-lim_y, lim_y);

        let fx = PK_W / 2.0 + self.pan_x - fw / 2.0;
        let fy = PK_H / 2.0 + self.pan_y - fh / 2.0;
        let x = fx.floor() as i32;
        let y = fy.floor() as i32;
        let w = MAX_W.min((fx + fw).ceil() as i32 - x);
        let h = MAX_H.min((fy + fh).ceil() as i32 - y);
        Some(Window {
            x,
            y,
            w,
            h,
            off_x: (x as f32 - fx) * self.scale,
            off_y: (y as f32 - fy) * self.scale,
            scale: self.scale,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_crisp_and_fit_the_largest_window() {
        // A 1080p view: 1x is 6.75 GBA px per screen px (the height), then 5, 4, 3 (2 would be 960 wide: too big).
        assert_eq!(levels_for(1920.0, 1080.0), [6.75, 5.0, 4.0, 3.0]);
        assert_eq!(levels_for(0.0, 100.0), [1.0]);
    }

    #[test]
    fn a_wide_view_shows_the_map_beside_the_frame() {
        let mut c = Camera::new();
        let w = c.frame(1920.0, 1080.0, SCENE_FIELD, 0.016).unwrap();
        // 1920 / 6.75 = 284.4 GBA px wide, centred on the frame.
        assert_eq!((w.y, w.h), (0, 160));
        assert!(w.x < 0 && w.w > 240 && w.x + w.w > 240, "{w:?}");
    }

    #[test]
    fn zooming_out_waits_for_the_field_and_a_battle_pulls_back_in() {
        let mut c = Camera::new();
        c.frame(1920.0, 1080.0, SCENE_FIELD, 0.016);
        assert!((c.cycle(1920.0, 1080.0) - 1.35).abs() < 0.01);
        // Eased out over ~0.65 s, then 5 px per GBA px.
        for _ in 0..90 {
            c.frame(1920.0, 1080.0, SCENE_FIELD, 0.016);
        }
        let w = c.frame(1920.0, 1080.0, SCENE_FIELD, 0.016).unwrap();
        assert_eq!(w.scale, 5.0);
        assert_eq!(
            (w.x, w.w),
            (-72, 384),
            "1920 / 5 = 384 GBA px, centred on the frame's 240"
        );
        for _ in 0..60 {
            c.frame(1920.0, 1080.0, SCENE_OFF, 0.016);
        }
        assert_eq!(
            c.frame(1920.0, 1080.0, SCENE_OFF, 0.016).unwrap().scale,
            6.75,
            "a battle is seen close up"
        );
    }

    #[test]
    fn looking_around_moves_the_view_and_walking_brings_it_back() {
        let mut c = Camera::new();
        // A game starts close up for a moment (RETURN_DELAY), as after a battle: the look-around waits it out.
        for _ in 0..40 {
            c.frame(1920.0, 1080.0, SCENE_FIELD, 0.016);
        }
        let start = c.frame(1920.0, 1080.0, SCENE_FIELD, 0.016).unwrap();
        c.look_x = 1.0;
        for _ in 0..60 {
            c.frame(1920.0, 1080.0, SCENE_FIELD, 0.016);
        }
        c.look_x = 0.0;
        let moved = c.frame(1920.0, 1080.0, SCENE_FIELD, 0.016).unwrap();
        assert!(moved.x > start.x + 100, "{moved:?} vs {start:?}");
        let held = c.frame(1920.0, 1080.0, SCENE_FIELD, 0.5).unwrap();
        assert_eq!(held.x, moved.x, "the view stays where it's left");
        c.walking = true;
        for _ in 0..120 {
            c.frame(1920.0, 1080.0, SCENE_FIELD, 0.016);
        }
        assert_eq!(
            c.frame(1920.0, 1080.0, SCENE_FIELD, 0.016).unwrap().x,
            start.x,
            "back on the player"
        );
    }
}
