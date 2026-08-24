// SPDX-License-Identifier: GPL-3.0-or-later
//! The graphical greeter: a Wayland client under `wlrix-compositor`.
//!
//! Two surfaces. The steel-blue backdrop is a layer-shell **background** surface, one
//! per output, covering each screen behind everything. The login dialog is an ordinary
//! xdg toplevel with `app_id` `com.wlrix.greeter`, which the compositor centers.
//!
//! The split is deliberate: this compositor composites layer surfaces but routes no
//! input to them, while a toplevel gets keyboard focus on map and pointer hit-testing.
//! So the part that needs input is the toplevel, and the backdrop -- which needs none --
//! is the layer surface.
//!
//! The dialog is drawn by [`dialog::paint`] from a [`dialog::Model`], and input is turned
//! into changes to that model by [`input`]. The event loop is `calloop`, so the caret
//! blink and scroll auto-repeat are timers alongside the Wayland source -- and greetd's
//! replies will slot in the same way.

pub mod dialog;
pub mod icons;
pub mod input;
pub mod layout;
pub mod preview;

use std::time::Duration;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::calloop::{
        EventLoop, LoopHandle, RegistrationToken,
        channel::Event as ChannelEvent,
        timer::{TimeoutAction, Timer},
    },
    reexports::calloop_wayland_source::WaylandSource,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{
        wl_keyboard::WlKeyboard, wl_output::WlOutput, wl_pointer::WlPointer, wl_seat::WlSeat,
        wl_shm, wl_surface::WlSurface,
    },
};

use crate::greetd::{Greetd, QuestionKind};
use crate::ui::dialog::{Focus, Model};
use crate::ui::input::{Action, Input, Key};
use crate::ui::layout::Geometry;
use crate::worker::{self, Command, Reply};
use wlrix_ui::Rgb;
use wlrix_ui::canvas::Canvas;
use wlrix_ui::palette::Palette;
use wlrix_ui::text::Fonts;

/// The dialog's fixed size, in logical pixels.
const DIALOG_W: u32 = 560;
const DIALOG_H: u32 = 470;

/// How long each half of the caret's blink lasts.
const CARET_BLINK: Duration = Duration::from_millis(530);
/// The pause before a held scroll arrow starts repeating, then the gap between repeats.
const REPEAT_DELAY: Duration = Duration::from_millis(400);
const REPEAT_INTERVAL: Duration = Duration::from_millis(60);

/// One screen's backdrop.
struct Backdrop {
    layer: LayerSurface,
    output: WlOutput,
    width: u32,
    height: u32,
}

/// Where a login attempt has got to. Controls what advancing does and whether input
/// is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No attempt running. Advancing begins one.
    PickUser,
    /// A request is out; greetd has not answered. Input is held off, so a keystroke
    /// does not land in a field about to be cleared -- PAM can take a second to reply.
    Waiting,
    /// A question is outstanding. Advancing answers it.
    Asking,
}

/// The greeter's whole UI state.
pub struct Greeter {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    shm: Shm,
    compositor: CompositorState,
    layer_shell: LayerShell,
    pool: SlotPool,
    loop_handle: LoopHandle<'static, Greeter>,

    backdrops: Vec<Backdrop>,
    window: Window,
    dialog_w: u32,
    dialog_h: u32,

    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,

    fonts: Fonts,
    /// The color scheme everything is drawn in. Chosen once, from `--palette`: the greeter
    /// is a short-lived screen started fresh per login, so there is nothing to reload into.
    palette: &'static Palette,
    model: Model,
    input: Input,
    worker: worker::Handle,
    phase: Phase,
    /// The running scroll-repeat timer, so it can be canceled on release.
    arrow_timer: Option<RegistrationToken>,

    /// Set when the model changed; the next draw repaints and clears it.
    dirty: bool,
    exit: bool,
}

/// Run the graphical greeter until the session starts or the dialog is closed.
///
/// `session` is what a successful login starts, and `username` pre-fills the field when
/// greetd named a user for us.
pub fn run(
    session: Vec<String>,
    username: Option<String>,
    palette: &'static Palette,
) -> Result<(), String> {
    // Connect to greetd first: a greeter that cannot authenticate is not worth showing,
    // and this fails the same way the terminal face does, before any window or font.
    let greetd = Greetd::connect()?;

    let mut fonts = Fonts::load()?;
    let cjk_ok = fonts.can_render("日本語");
    eprintln!(
        "wlrix-greeter: drawing with {} ({} faces, cjk sample renders: {})",
        fonts.family(),
        fonts.face_count(),
        cjk_ok
    );

    let conn = Connection::connect_to_env()
        .map_err(|err| format!("no Wayland compositor to connect to: {err}"))?;
    let (globals, event_queue) =
        registry_queue_init(&conn).map_err(|err| format!("could not read the registry: {err}"))?;
    let qh = event_queue.handle();

    let mut event_loop: EventLoop<Greeter> =
        EventLoop::try_new().map_err(|err| format!("could not create the event loop: {err}"))?;
    let loop_handle = event_loop.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|err| format!("wl_compositor unavailable: {err}"))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|err| format!("wlr-layer-shell unavailable: {err}"))?;
    let xdg_shell =
        XdgShell::bind(&globals, &qh).map_err(|err| format!("xdg-shell unavailable: {err}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|err| format!("wl_shm unavailable: {err}"))?;

    let pool = SlotPool::new((DIALOG_W * DIALOG_H * 4) as usize, &shm)
        .map_err(|err| format!("could not create a buffer pool: {err}"))?;

    // The dialog. A bare surface -- the compositor draws no chrome.
    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestClient, &qh);
    window.set_title("wlRIX");
    window.set_app_id("com.wlrix.greeter");
    window.set_min_size(Some((DIALOG_W, DIALOG_H)));
    window.set_max_size(Some((DIALOG_W, DIALOG_H)));
    window.commit();

    let mut model = Model::new(crate::users::list());
    // A named user pre-fills the field, ready to advance.
    if let Some(name) = &username {
        model.username = name.clone();
        model.focus = Focus::Username;
    }

    // The greetd conversation runs on its own thread; its replies arrive on a channel
    // the loop waits on, so a slow PAM never freezes the screen.
    let (worker, replies) = worker::spawn(greetd, session);

    let mut greeter = Greeter {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        shm,
        compositor,
        layer_shell,
        pool,
        loop_handle: loop_handle.clone(),
        palette,
        backdrops: Vec::new(),
        window,
        dialog_w: DIALOG_W,
        dialog_h: DIALOG_H,
        keyboard: None,
        pointer: None,
        fonts,
        model,
        input: Input::default(),
        worker,
        phase: Phase::PickUser,
        arrow_timer: None,
        dirty: true,
        exit: false,
    };

    WaylandSource::new(conn, event_queue)
        .insert(loop_handle.clone())
        .map_err(|err| format!("could not drive Wayland from the loop: {err}"))?;

    loop_handle
        .insert_source(replies, |event, _, greeter| {
            match event {
                ChannelEvent::Msg(reply) => greeter.on_reply(reply),
                // The worker thread is gone. That should not happen while the greeter is
                // still up, so show it rather than vanishing to a bare desktop -- a
                // greeter that disappears strands whoever is at the keyboard.
                ChannelEvent::Closed => {
                    if !greeter.exit {
                        greeter.model.message = Some("lost the connection to greetd".to_string());
                        greeter.phase = Phase::PickUser;
                        greeter.request_redraw();
                    }
                }
            }
        })
        .map_err(|err| format!("could not receive greetd replies: {err}"))?;

    // The caret blink runs for the life of the greeter, toggling and redrawing.
    loop_handle
        .insert_source(Timer::from_duration(CARET_BLINK), |_, _, greeter| {
            greeter.model.caret_on = !greeter.model.caret_on;
            greeter.request_redraw();
            TimeoutAction::ToDuration(CARET_BLINK)
        })
        .map_err(|err| format!("could not start the caret timer: {err}"))?;

    // A named user is logged straight in: send the request now, and the reply drives
    // the rest as if the button had been pressed.
    if username.is_some() {
        greeter.submit();
    }

    // Backdrops depend on the output list, which the loop delivers; build any that are
    // known already, and `new_output` covers the rest.
    let qh2 = qh.clone();
    event_loop
        .dispatch(Duration::from_millis(200), &mut greeter)
        .map_err(|err| format!("initial dispatch failed: {err}"))?;
    greeter.rebuild_backdrops(&qh2);

    loop {
        event_loop
            .dispatch(Duration::from_secs(1), &mut greeter)
            .map_err(|err| format!("event loop failed: {err}"))?;
        // Paint after every batch of events, rather than waiting on a frame callback.
        // A login screen redraws on demand -- a click, a keystroke, the caret blink --
        // and a frame callback only arrives after a commit that requested one, so
        // gating drawing on it stalls the moment nothing is being drawn. The caret
        // timer and input both wake this loop, so a change is on screen promptly.
        greeter.draw_if_dirty();
        if greeter.exit {
            return Ok(());
        }
    }
}

impl Greeter {
    fn request_redraw(&mut self) {
        // The dialog is small and painted wholesale, so this just flags it; the running
        // loop paints once per turn when dirty.
        self.dirty = true;
    }

    /// Paint the dialog if anything changed since the last frame.
    fn draw_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.draw_dialog();
    }

    /// Make a backdrop for every output that lacks one, and drop any whose output is
    /// gone. Outputs are hotpluggable, so this runs whenever the set changes.
    fn rebuild_backdrops(&mut self, qh: &QueueHandle<Self>) {
        let outputs: Vec<WlOutput> = self.output_state.outputs().collect();
        self.backdrops
            .retain(|backdrop| outputs.contains(&backdrop.output));

        for output in outputs {
            if self.backdrops.iter().any(|b| b.output == output) {
                continue;
            }
            let surface = self.compositor.create_surface(qh);
            let layer = self.layer_shell.create_layer_surface(
                qh,
                surface,
                Layer::Background,
                Some("wlrix-greeter-backdrop"),
                Some(&output),
            );
            layer.set_anchor(Anchor::all());
            layer.set_exclusive_zone(-1);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.set_size(0, 0);
            layer.commit();
            self.backdrops.push(Backdrop {
                layer,
                output,
                width: 0,
                height: 0,
            });
        }
    }

    /// Fill a backdrop with the login-screen blue.
    fn draw_backdrop(&mut self, index: usize) {
        let Some(backdrop) = self.backdrops.get(index) else {
            return;
        };
        let (width, height) = (backdrop.width.max(1), backdrop.height.max(1));
        let stride = width as i32 * 4;
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) else {
            return;
        };
        fill(canvas, self.palette.login_background);
        let layer = self.backdrops[index].layer.wl_surface();
        layer.damage_buffer(0, 0, width as i32, height as i32);
        if buffer.attach_to(layer).is_ok() {
            layer.commit();
        }
    }

    /// Paint the login dialog from the model.
    fn draw_dialog(&mut self) {
        let (width, height) = (self.dialog_w.max(1), self.dialog_h.max(1));
        let stride = width as i32 * 4;
        let Ok((buffer, pixels)) = self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) else {
            return;
        };
        let mut canvas = Canvas::new(pixels, width as i32, height as i32);
        dialog::paint(&mut canvas, self.palette, &mut self.fonts, &self.model);

        let surface = self.window.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        if buffer.attach_to(surface).is_ok() {
            surface.commit();
        }
    }

    /// Act on a keystroke or click that the input layer reported.
    fn perform(&mut self, action: Action) {
        match action {
            Action::None => {}
            Action::Advance => self.submit(),
            Action::HoldArrow(up) => self.hold_arrow(up),
        }
    }

    /// Advance the login: begin an attempt, or answer the outstanding question.
    ///
    /// The greeter never decides whether a password is wanted -- it asks greetd to begin
    /// and waits. A password field appears only when [`Self::on_reply`] receives a
    /// question, never here.
    fn submit(&mut self) {
        match self.phase {
            Phase::PickUser => {
                let username = self.model.username.trim().to_string();
                if username.is_empty() {
                    return;
                }
                self.model.message = None;
                self.phase = Phase::Waiting;
                self.worker.send(Command::Begin { username });
            }
            Phase::Asking => {
                let answer = self.model.password.clone().unwrap_or_default();
                self.phase = Phase::Waiting;
                self.worker.send(Command::Answer(answer));
            }
            // A request is already out; ignore until greetd answers.
            Phase::Waiting => {}
        }
        self.request_redraw();
    }

    /// Act on what the greetd conversation said.
    fn on_reply(&mut self, reply: Reply) {
        match reply {
            Reply::Question(question) => {
                // A question -- and only a question -- reveals the answer field. Its
                // kind decides whether the answer is hidden as it is typed.
                self.model.password = Some(String::new());
                self.model.password_secret = question.kind == QuestionKind::Secret;
                self.model.focus = Focus::Password;
                self.model.message = None;
                self.phase = Phase::Asking;
            }
            Reply::Notice(text) => {
                // Shown, but the worker answers PAM itself and carries on, so the phase
                // does not change.
                self.model.message = Some(text);
            }
            Reply::Started => {
                // The session is running. greetd tears the greeter down, but exit
                // cleanly in case it does not get to.
                self.exit = true;
            }
            Reply::Denied(_) => {
                // IRIX showed a modal box on a failed login rather than an inline note.
                // The reason from PAM is deliberately not shown -- "which of username or
                // password was wrong" is exactly what an attacker wants narrowed down.
                self.model.error = Some("Cannot log in to this account.".to_string());
                self.model.password = None;
                self.phase = Phase::PickUser;
            }
            Reply::Broken(err) => {
                self.model.error = Some(format!("Cannot log in to this account.\n{err}"));
                self.model.password = None;
                self.phase = Phase::PickUser;
            }
        }
        self.request_redraw();
    }

    /// Whether input should be acted on. A request in flight holds it off, so a
    /// keystroke does not edit a field about to be replaced.
    fn accepts_input(&self) -> bool {
        self.phase != Phase::Waiting
    }

    /// Start (or restart) the scroll-arrow auto-repeat.
    fn hold_arrow(&mut self, up: bool) {
        self.stop_arrow();
        let token = self.loop_handle.insert_source(
            Timer::from_duration(REPEAT_DELAY),
            move |_, _, greeter| {
                let g = Geometry::compute(
                    greeter.palette,
                    greeter.dialog_w as i32,
                    greeter.dialog_h as i32,
                );
                if input::repeat_arrow(&mut greeter.model, &g, up) {
                    greeter.request_redraw();
                    TimeoutAction::ToDuration(REPEAT_INTERVAL)
                } else {
                    // Reached the end of the list; stop until pressed again.
                    greeter.arrow_timer = None;
                    TimeoutAction::Drop
                }
            },
        );
        self.arrow_timer = token.ok();
    }

    /// Stop any running scroll-arrow repeat.
    fn stop_arrow(&mut self) {
        if let Some(token) = self.arrow_timer.take() {
            self.loop_handle.remove(token);
        }
    }

    /// The pointer position in dialog coordinates. The toplevel surface is the dialog,
    /// so surface coordinates already are dialog coordinates.
    fn geometry(&self) -> Geometry {
        Geometry::compute(self.palette, self.dialog_w as i32, self.dialog_h as i32)
    }
}

/// Paint every pixel of a raw buffer one color.
///
/// Not [`wlrix_ui::Canvas::clear`]: the backdrop is a bare `wl_shm` buffer with no drawing
/// on it at all, and wrapping it in a canvas to fill it once would be the only reason that
/// canvas existed.
fn fill(canvas: &mut [u8], color: Rgb) {
    let bytes = color.0.to_ne_bytes();
    // The remainder is discarded: a `wl_shm` buffer is whole pixels by construction.
    for pixel in canvas.as_chunks_mut::<4>().0 {
        *pixel = bytes;
    }
}

impl CompositorHandler for Greeter {
    fn scale_factor_changed(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: &WlSurface,
        _f: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: &WlSurface,
        _t: wayland_client::protocol::wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _c: &Connection, _qh: &QueueHandle<Self>, _surface: &WlSurface, _t: u32) {
        // Drawing is driven from the event loop after each dispatch, not from frame
        // callbacks -- the greeter never requests one. See the draw call in `run`.
    }
    fn surface_enter(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: &WlSurface,
        _o: &WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: &WlSurface,
        _o: &WlOutput,
    ) {
    }
}

impl LayerShellHandler for Greeter {
    fn closed(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _l: &LayerSurface) {}

    fn configure(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(index) = self.backdrops.iter().position(|b| &b.layer == layer) else {
            return;
        };
        let (width, height) = configure.new_size;
        self.backdrops[index].width = width;
        self.backdrops[index].height = height;
        self.draw_backdrop(index);
    }
}

impl WindowHandler for Greeter {
    fn request_close(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _w: &Window) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _c: &Connection,
        _qh: &QueueHandle<Self>,
        _w: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        if let (Some(w), Some(h)) = configure.new_size {
            self.dialog_w = w.get();
            self.dialog_h = h.get();
        }
        self.dirty = true;
        self.draw_dialog();
    }
}

impl SeatHandler for Greeter {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _s: WlSeat) {}

    fn new_capability(
        &mut self,
        _c: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && self.keyboard.is_none()
            && let Ok(kbd) = self.seat_state.get_keyboard(qh, &seat, None)
        {
            self.keyboard = Some(kbd);
        }
        if capability == Capability::Pointer
            && self.pointer.is_none()
            && let Ok(ptr) = self.seat_state.get_pointer(qh, &seat)
        {
            self.pointer = Some(ptr);
        }
    }

    fn remove_capability(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && let Some(kbd) = self.keyboard.take()
        {
            kbd.release();
        }
        if capability == Capability::Pointer
            && let Some(ptr) = self.pointer.take()
        {
            ptr.release();
        }
    }

    fn remove_seat(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _s: WlSeat) {}
}

impl KeyboardHandler for Greeter {
    fn enter(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _kbd: &WlKeyboard,
        _surface: &WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
        // Keyboard focus arrived; start the caret in the field, if one is chosen.
        self.model.caret_on = true;
        self.request_redraw();
    }

    fn leave(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _kbd: &WlKeyboard,
        _surface: &WlSurface,
        _serial: u32,
    ) {
        self.model.caret_on = false;
        self.request_redraw();
    }

    fn press_key(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _kbd: &WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if !self.accepts_input() {
            return;
        }
        let key = match event.keysym {
            Keysym::Return | Keysym::KP_Enter => Key::Enter,
            Keysym::BackSpace => Key::Backspace,
            Keysym::Tab => Key::Tab,
            _ => match event.utf8 {
                Some(text) => Key::Text(text),
                None => Key::Other,
            },
        };
        let action = input::key(&mut self.model, key);
        // A keystroke resets the caret to visible, so it does not blink out mid-type.
        self.model.caret_on = true;
        self.perform(action);
        self.request_redraw();
    }

    fn release_key(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _kbd: &WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _kbd: &WlKeyboard,
        _serial: u32,
        _modifiers: Modifiers,
        _layout: u32,
    ) {
    }
}

impl PointerHandler for Greeter {
    fn pointer_frame(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _pointer: &WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            // Only the dialog surface carries the widgets; the backdrop takes no input.
            if &event.surface != self.window.wl_surface() {
                continue;
            }
            let (x, y) = (event.position.0 as i32, event.position.1 as i32);
            let g = self.geometry();
            match event.kind {
                PointerEventKind::Press { button, time, .. }
                    if button == BTN_LEFT && self.accepts_input() =>
                {
                    let action = input::press(&mut self.model, &mut self.input, &g, x, y, time);
                    self.perform(action);
                    self.request_redraw();
                }
                PointerEventKind::Release { button, .. } if button == BTN_LEFT => {
                    let action = input::release(&mut self.model, &mut self.input, &g, x, y);
                    self.perform(action);
                    self.stop_arrow();
                    self.request_redraw();
                }
                PointerEventKind::Motion { .. } => {
                    input::motion(&mut self.model, &mut self.input, &g, x, y);
                    self.request_redraw();
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let delta = vertical.discrete;
                    if delta != 0 {
                        input::wheel(&mut self.model, &g, delta);
                        self.request_redraw();
                    }
                }
                _ => {}
            }
        }
    }
}

/// The Linux button code for the left mouse button.
const BTN_LEFT: u32 = 0x110;

impl ShmHandler for Greeter {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl OutputHandler for Greeter {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _c: &Connection, qh: &QueueHandle<Self>, _o: WlOutput) {
        self.rebuild_backdrops(qh);
    }
    fn update_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: WlOutput) {}
    fn output_destroyed(&mut self, _c: &Connection, qh: &QueueHandle<Self>, _o: WlOutput) {
        self.rebuild_backdrops(qh);
    }
}

impl ProvidesRegistryState for Greeter {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Greeter);
delegate_output!(Greeter);
delegate_shm!(Greeter);
delegate_seat!(Greeter);
delegate_keyboard!(Greeter);
delegate_pointer!(Greeter);
delegate_layer!(Greeter);
delegate_xdg_shell!(Greeter);
delegate_xdg_window!(Greeter);
delegate_registry!(Greeter);
