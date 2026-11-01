//! Touch controls (Android): the left part of the screen is a virtual
//! joystick, the right part a look surface, taps and holds act as mouse
//! buttons, and round buttons overlay the corners. Everything is routed
//! through what the game already understands, so keybinds, sensitivity and
//! toggle options apply unchanged:
//!
//! - the joystick is the virtual pad's left stick (`pad.injected.left`,
//!   the same state `horizon-ctl pad` writes), moving the player analog
//!   through `pad_move`; pushing it past ~95% sprints (the stick click);
//! - look drags are the right stick, so `pad_look`'s sensitivity, invert
//!   and spyglass rules apply;
//! - the overlay buttons are pad buttons (A jump, RightStick sneak, Y
//!   inventory, B drop, Start pause): their presses go through the same
//!   edges and vanilla key mappings a controller's do;
//! - a quick tap on the look surface is a right click (use/place), and
//!   holding it is a held left click (mine/attack), like Bedrock's;
//! - over menus (pause, inventory, the title) a touch is a cursor and a
//!   tap is a click, synthesized as the mouse events the screens eat.

use super::{Game, Screen};
use horizon_client::gamepad::Button;
use horizon_client::hud::{HudAtlas, HudBatch, rgba};
use std::time::{Duration, Instant};
use winit::event::{DeviceId, ElementState, MouseButton, TouchPhase, WindowEvent};

/// How a touch was answered: the normal window-event path stops for
/// [`Outcome::Handled`], runs the synthetic events of [`Outcome::Events`]
/// through `handle_window_event` as if the window had sent them, or lets
/// the event through untouched ([`Outcome::Pass`], an extra finger).
pub(super) enum Outcome {
    Handled,
    Events(Vec<WindowEvent>),
    Pass,
}

/// A quick tap on the look surface (shorter and smaller than this) is a
/// right click rather than part of a drag.
const TAP_MAX: Duration = Duration::from_millis(350);
const TAP_DIST_DP: f32 = 14.0;
/// A look-hold longer than this mines (the held left click).
const HOLD_MINING: Duration = Duration::from_millis(220);
/// How long a synthesized press stays down before its release is sent.
const RELEASE_AFTER: Duration = Duration::from_millis(60);
/// Joystick radius, in dp.
const STICK_RADIUS_DP: f32 = 56.0;
/// Drag pixels (per frame, scaled to dp) that count as a full right-stick
/// tilt: 6 dp/frame at 60 fps is a brisk swipe.
const LOOK_DP_PER_TILT: f32 = 6.0;
/// The stick pushes past this to sprint (the stick click).
const SPRINT_TILT: f32 = 0.95;

/// One finger on the joystick.
struct Stick {
    id: u64,
    /// Where the finger went down: the joystick's centre.
    origin: [f32; 2],
    pos: [f32; 2],
}

/// One finger on the look surface.
struct Look {
    id: u64,
    last: [f32; 2],
    down_at: Instant,
    moved: f32,
}

/// The on-screen buttons, in dp, drawn over the HUD while playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Jump,
    Sneak,
    Drop,
    Inventory,
    Pause,
}

const CONTROLS: [Control; 5] = [
    Control::Jump,
    Control::Sneak,
    Control::Drop,
    Control::Inventory,
    Control::Pause,
];

impl Control {
    /// The pad button the control presses, through the vanilla key mappings
    /// (A jump, RightStick sneak, B drop, Y inventory, Start pause).
    fn button(self) -> Button {
        match self {
            Control::Jump => Button::South,
            Control::Sneak => Button::RightStick,
            Control::Drop => Button::East,
            Control::Inventory => Button::North,
            Control::Pause => Button::Start,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Control::Jump => "A",
            Control::Sneak => "S",
            Control::Drop => "Q",
            Control::Inventory => "E",
            Control::Pause => "M",
        }
    }

    fn radius_dp(self) -> f32 {
        match self {
            Control::Jump => 42.0,
            Control::Sneak => 34.0,
            Control::Drop => 30.0,
            Control::Inventory => 30.0,
            Control::Pause => 26.0,
        }
    }

    /// The centre in dp, from the screen's dp size.
    fn center_dp(self, w_dp: f32, h_dp: f32) -> [f32; 2] {
        match self {
            Control::Jump => [w_dp - 76.0, h_dp - 84.0],
            Control::Sneak => [w_dp - 76.0, h_dp - 194.0],
            Control::Drop => [w_dp - 192.0, h_dp - 84.0],
            Control::Inventory => [w_dp - 48.0, 80.0],
            Control::Pause => [48.0, 48.0],
        }
    }

    /// Hit test in px, with a finger's slop around the drawn size.
    fn hit(self, x: f32, y: f32, w_dp: f32, h_dp: f32, scale: f32) -> bool {
        let [cx, cy] = self.center_dp(w_dp, h_dp);
        let (dx, dy) = (x / scale - cx, y / scale - cy);
        (dx * dx + dy * dy).sqrt() <= self.radius_dp() * 1.2
    }
}

/// The state of every finger the controls have taken.
#[derive(Default)]
pub(super) struct Touch {
    stick: Option<Stick>,
    look: Option<Look>,
    /// Fingers holding an on-screen button down, and which.
    held: Vec<(u64, Control)>,
    /// Pad buttons a tap pressed; the next frame releases them.
    tap_release: Vec<Button>,
    /// The look-hold's virtual left click, still down.
    mining: bool,
    /// Synthesized mouse presses whose release is due.
    pending_release: Vec<(MouseButton, Instant)>,
    /// Look drag accumulated since the last frame, in px (x right, y down).
    look_delta: [f32; 2],
}

impl Touch {
    /// A touch event, before the game's own input handling sees it.
    pub(super) fn on_event(&mut self, game: &mut Game, event: &WindowEvent) -> Outcome {
        let WindowEvent::Touch(winit::event::Touch {
            id,
            phase,
            location,
            ..
        }) = event
        else {
            return Outcome::Pass;
        };
        let (x, y) = (location.x as f32, location.y as f32);
        if game.pad_in_menu() {
            // Menus are driven by the mouse: the touch becomes a cursor move
            // and a click, as if a pointer had done it.
            let device = DeviceId::dummy();
            let position = winit::dpi::PhysicalPosition::new(f64::from(x), f64::from(y));
            let events = match phase {
                TouchPhase::Started => vec![
                    WindowEvent::CursorMoved {
                        device_id: device,
                        position,
                    },
                    WindowEvent::MouseInput {
                        device_id: device,
                        state: ElementState::Pressed,
                        button: MouseButton::Left,
                    },
                ],
                TouchPhase::Moved => {
                    vec![WindowEvent::CursorMoved {
                        device_id: device,
                        position,
                    }]
                }
                TouchPhase::Ended => vec![WindowEvent::MouseInput {
                    device_id: device,
                    state: ElementState::Released,
                    button: MouseButton::Left,
                }],
                TouchPhase::Cancelled => vec![],
            };
            Outcome::Events(events)
        } else {
            self.play_touch(game, *id, *phase, x, y)
        }
    }

    /// Gameplay: the joystick, the look surface and the buttons.
    fn play_touch(
        &mut self,
        game: &mut Game,
        id: u64,
        phase: TouchPhase,
        x: f32,
        y: f32,
    ) -> Outcome {
        let (w_dp, h_dp, scale) = screen_dp(game);
        match phase {
            TouchPhase::Started => {
                for control in CONTROLS {
                    if control.hit(x, y, w_dp, h_dp, scale) {
                        self.held.push((id, control));
                        game.pad.injected.set(control.button(), true);
                        return Outcome::Handled;
                    }
                }
                if x / scale < w_dp * 0.45 && y / scale > h_dp * 0.3 {
                    self.stick = Some(Stick {
                        id,
                        origin: [x, y],
                        pos: [x, y],
                    });
                    return Outcome::Handled;
                }
                self.look = Some(Look {
                    id,
                    last: [x, y],
                    down_at: Instant::now(),
                    moved: 0.0,
                });
                Outcome::Handled
            }
            TouchPhase::Moved => {
                if let Some(stick) = self.stick.as_mut()
                    && stick.id == id
                {
                    stick.pos = [x, y];
                    return Outcome::Handled;
                }
                if let Some(look) = self.look.as_mut()
                    && look.id == id
                {
                    let (dx, dy) = (x - look.last[0], y - look.last[1]);
                    self.look_delta[0] += dx;
                    self.look_delta[1] += dy;
                    look.moved += (dx * dx + dy * dy).sqrt();
                    look.last = [x, y];
                    return Outcome::Handled;
                }
                Outcome::Pass
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                if let Some(at) = self.held.iter().position(|(held, _)| *held == id) {
                    let (_, control) = self.held.remove(at);
                    game.pad.injected.set(control.button(), false);
                    return Outcome::Handled;
                }
                if self.stick.as_ref().is_some_and(|s| s.id == id) {
                    self.stick = None;
                    game.pad.injected.left = [0.0, 0.0];
                    game.pad.injected.set(Button::LeftStick, false);
                    return Outcome::Handled;
                }
                if self.look.as_ref().is_some_and(|l| l.id == id) {
                    let look = self.look.take().expect("checked");
                    let mining = self.mining;
                    self.mining = false;
                    if mining {
                        let _ = game.play_mouse(MouseButton::Left, false);
                    } else if phase == TouchPhase::Ended
                        && look.down_at.elapsed() < TAP_MAX
                        && look.moved / scale < TAP_DIST_DP
                    {
                        // A quick tap: use / place (the right click), released
                        // by the next frames.
                        let _ = game.play_mouse(MouseButton::Right, true);
                        self.pending_release
                            .push((MouseButton::Right, Instant::now()));
                    }
                    return Outcome::Handled;
                }
                Outcome::Pass
            }
        }
    }

    /// Once per loop iteration (`idle`): the sticks from the fingers' current
    /// places, the look-hold's mining, and the due releases.
    pub(super) fn frame(&mut self, game: &mut Game) {
        if let Some(stick) = &self.stick {
            let (_, _, scale) = screen_dp(game);
            let radius = STICK_RADIUS_DP * scale;
            let (dx, dy) = (
                (stick.pos[0] - stick.origin[0]) / radius,
                (stick.pos[1] - stick.origin[1]) / radius,
            );
            let len = (dx * dx + dy * dy).sqrt();
            let (vx, vy) = if len > 1.0 {
                (dx / len, dy / len)
            } else {
                (dx, dy)
            };
            // A real stick reports x right, y up; the screen's y is down.
            game.pad.injected.left = [vx.clamp(-1.0, 1.0), (-vy).clamp(-1.0, 1.0)];
            game.pad.injected.set(Button::LeftStick, len > SPRINT_TILT);
        }
        if let Some(look) = &self.look {
            let (_, _, scale) = screen_dp(game);
            let divide = LOOK_DP_PER_TILT * scale;
            game.pad.injected.right = [
                (self.look_delta[0] / divide).clamp(-1.0, 1.0),
                (-self.look_delta[1] / divide).clamp(-1.0, 1.0),
            ];
            if !self.mining && look.down_at.elapsed() > HOLD_MINING {
                let _ = game.play_mouse(MouseButton::Left, true);
                self.mining = true;
            }
        } else {
            game.pad.injected.right = [0.0, 0.0];
        }
        self.look_delta = [0.0, 0.0];
        // Buttons a tap pressed go up again once a poll has seen them down.
        for button in std::mem::take(&mut self.tap_release) {
            game.pad.injected.set(button, false);
        }
        let due: Vec<(MouseButton, Instant)> = self
            .pending_release
            .iter()
            .copied()
            .filter(|(_, at)| at.elapsed() >= RELEASE_AFTER)
            .collect();
        for (button, _) in due {
            let _ = game.play_mouse(button, false);
            self.pending_release.retain(|(b, _)| *b != button);
        }
    }

    /// The buttons and the joystick, over the HUD (`hud`'s last layer).
    pub(super) fn draw(&self, game: &Game, batch: &mut HudBatch) {
        if game.pad_in_menu() || game.screen != Screen::Playing {
            return;
        }
        let atlas: &HudAtlas = &game.gui.atlas;
        let scale = batch.scale;
        let (w_dp, h_dp) = (batch.width / scale, batch.height / scale);
        let plate = rgba(255, 255, 255, 74);
        let plate_held = rgba(255, 255, 255, 140);
        let ink = rgba(255, 255, 255, 190);
        for control in CONTROLS {
            let [cx, cy] = control.center_dp(w_dp, h_dp);
            let r = control.radius_dp();
            let held = self.held.iter().any(|(_, c)| *c == control);
            ring(
                batch,
                atlas,
                cx * scale,
                cy * scale,
                r * scale,
                if held { 7.0 } else { 4.0 } * scale,
                if held { plate_held } else { plate },
            );
            let (x, y) = (cx * scale, cy * scale);
            let width = HudBatch::text_width(atlas, control.label());
            batch.text(atlas, control.label(), x - width / 2.0, y, ink, true);
        }
        if let Some(stick) = &self.stick {
            ring(
                batch,
                atlas,
                stick.origin[0],
                stick.origin[1],
                STICK_RADIUS_DP * scale,
                3.0 * scale,
                plate,
            );
            let (dx, dy) = (
                stick.pos[0] - stick.origin[0],
                stick.pos[1] - stick.origin[1],
            );
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let reach = (STICK_RADIUS_DP * scale).min(len);
            let (kx, ky) = (
                stick.origin[0] + dx / len * reach,
                stick.origin[1] + dy / len * reach,
            );
            disc(batch, atlas, kx, ky, 20.0 * scale, rgba(255, 255, 255, 96));
        }
    }
}

/// The screen's size in dp, from the renderer (the window's own size in the
/// frames before it exists).
fn screen_dp(game: &Game) -> (f32, f32, f32) {
    let size = game
        .renderer
        .as_ref()
        .map(|r| [r.config.width, r.config.height])
        .or_else(|| {
            game.window
                .as_ref()
                .map(|w| w.inner_size())
                .map(|s| [s.width, s.height])
        })
        .unwrap_or([1280, 720]);
    let batch = HudBatch::new(size);
    (
        batch.width / batch.scale,
        batch.height / batch.scale,
        batch.scale,
    )
}

/// A ring, as segments of one colour: the joystick's well and the buttons.
fn ring(batch: &mut HudBatch, atlas: &HudAtlas, cx: f32, cy: f32, r: f32, t: f32, color: u32) {
    let steps = 24;
    let at = |a: f32, radius: f32| [cx + radius * a.cos(), cy + radius * a.sin()];
    for i in 0..steps {
        let a0 = std::f32::consts::TAU * i as f32 / steps as f32;
        let a1 = std::f32::consts::TAU * (i + 1) as f32 / steps as f32;
        batch.fill_quad(
            atlas,
            [at(a0, r - t), at(a0, r), at(a1, r), at(a1, r - t)],
            color,
        );
    }
}

/// A filled disc (the joystick's knob): a fan from the centre.
fn disc(batch: &mut HudBatch, atlas: &HudAtlas, cx: f32, cy: f32, r: f32, color: u32) {
    ring(batch, atlas, cx, cy, r, r, color)
}

impl Game<'_> {
    /// Once per loop iteration (`idle`, after the pads are polled): the
    /// sticks from the fingers' places, the look-hold, the due releases.
    #[cfg(target_os = "android")]
    pub(super) fn touch_frame(&mut self) {
        let mut touch = std::mem::take(&mut self.touch);
        touch.frame(self);
        self.touch = touch;
    }

    /// The touch controls drawn over the HUD (its last layer).
    #[cfg(target_os = "android")]
    pub(super) fn draw_touch_controls(&self, batch: &mut HudBatch) {
        self.touch.draw(self, batch);
    }
}
