// SPDX-License-Identifier: GPL-3.0-or-later
//! The graphical greeter: a Wayland client under `wlrix-compositor`.
//!
//! Two surfaces. The steel-blue backdrop is a layer-shell **background** surface, one
//! per output, covering each screen behind everything. The login dialog is an ordinary
//! xdg toplevel with `app_id` `com.wlrix.greeter`, which the compositor centres.
//!
//! The split is deliberate: this compositor composites layer surfaces but routes no
//! input to them, while a toplevel gets keyboard focus on map and pointer hit-testing.
//! So the part that needs input is the toplevel, and the backdrop -- which needs none --
//! is the layer surface.
//!
//! This stage only brings the surfaces up and fills them flat. Drawing the dialog comes
//! next.

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_seat,
    delegate_shm, delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{Capability, SeatHandler, SeatState},
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
    protocol::{wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface},
};

use crate::theme::{Rgb, palette};

/// The dialog's fixed size, in logical pixels. Close to IRIX's clogin; the exact layout
/// is settled when the widgets land.
const DIALOG_W: u32 = 560;
const DIALOG_H: u32 = 470;

/// One screen's backdrop.
struct Backdrop {
    layer: LayerSurface,
    output: WlOutput,
    width: u32,
    height: u32,
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

    /// One backdrop per output; outputs can come and go, so this is rebuilt on hotplug.
    backdrops: Vec<Backdrop>,
    /// The login dialog.
    window: Window,
    dialog_w: u32,
    dialog_h: u32,

    /// Set when the compositor asks the toplevel to close, or a fatal error occurs.
    exit: bool,
}

/// Run the graphical greeter until the session starts or the dialog is closed.
pub fn run() -> Result<(), String> {
    let conn = Connection::connect_to_env()
        .map_err(|err| format!("no Wayland compositor to connect to: {err}"))?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|err| format!("could not read the registry: {err}"))?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|err| format!("wl_compositor unavailable: {err}"))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|err| format!("wlr-layer-shell unavailable: {err}"))?;
    let xdg_shell =
        XdgShell::bind(&globals, &qh).map_err(|err| format!("xdg-shell unavailable: {err}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|err| format!("wl_shm unavailable: {err}"))?;

    // Big enough for the dialog at 4 bytes a pixel; the pool grows itself if a backdrop
    // needs more.
    let pool = SlotPool::new((DIALOG_W * DIALOG_H * 4) as usize, &shm)
        .map_err(|err| format!("could not create a buffer pool: {err}"))?;

    // The dialog. A bare surface -- the compositor draws no chrome (it hands clients
    // client-side decorations), which is what the reference shows.
    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestClient, &qh);
    window.set_title("wlRIX");
    window.set_app_id("com.wlrix.greeter");
    // A fixed size, so the compositor's centring rule has something real to place.
    window.set_min_size(Some((DIALOG_W, DIALOG_H)));
    window.set_max_size(Some((DIALOG_W, DIALOG_H)));
    window.commit();

    let mut greeter = Greeter {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        shm,
        compositor,
        layer_shell,
        pool,
        backdrops: Vec::new(),
        window,
        dialog_w: DIALOG_W,
        dialog_h: DIALOG_H,
        exit: false,
    };

    // The backdrops depend on the output list, which arrives on the first roundtrip.
    event_queue
        .roundtrip(&mut greeter)
        .map_err(|err| format!("initial roundtrip failed: {err}"))?;
    greeter.rebuild_backdrops(&qh);

    loop {
        event_queue
            .blocking_dispatch(&mut greeter)
            .map_err(|err| format!("event loop failed: {err}"))?;
        if greeter.exit {
            return Ok(());
        }
    }
}

impl Greeter {
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
            // Cover the whole output, claim no exclusive zone, take no input -- the
            // dialog handles that.
            layer.set_anchor(Anchor::all());
            layer.set_exclusive_zone(-1);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            // Zero size means "as large as the anchors imply", i.e. the whole output.
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
    fn draw_backdrop(&mut self, index: usize, qh: &QueueHandle<Self>) {
        let Some(backdrop) = self.backdrops.get(index) else {
            return;
        };
        let (width, height) = (backdrop.width.max(1), backdrop.height.max(1));
        let stride = width as i32 * 4;

        let (buffer, canvas) = match self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) {
            Ok(pair) => pair,
            Err(_) => return,
        };
        fill(canvas, palette::LOGIN_BACKGROUND);

        let layer = self.backdrops[index].layer.wl_surface();
        layer.damage_buffer(0, 0, width as i32, height as i32);
        layer.frame(qh, layer.clone());
        if let Err(err) = buffer.attach_to(layer) {
            tracing_note(&format!("could not attach backdrop buffer: {err}"));
        }
        layer.commit();
    }

    /// Fill the dialog. Flat for now; the widgets replace this.
    fn draw_dialog(&mut self, qh: &QueueHandle<Self>) {
        let (width, height) = (self.dialog_w.max(1), self.dialog_h.max(1));
        let stride = width as i32 * 4;

        let (buffer, canvas) = match self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) {
            Ok(pair) => pair,
            Err(_) => return,
        };
        // The panel grey, so the dialog is visibly distinct from the backdrop even
        // before any furniture is drawn.
        fill(canvas, palette::PANEL);

        let surface = self.window.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        surface.frame(qh, surface.clone());
        if let Err(err) = buffer.attach_to(surface) {
            tracing_note(&format!("could not attach dialog buffer: {err}"));
        }
        surface.commit();
    }
}

/// Paint every pixel of a canvas one colour.
fn fill(canvas: &mut [u8], colour: Rgb) {
    let bytes = colour.0.to_ne_bytes();
    for pixel in canvas.chunks_exact_mut(4) {
        pixel.copy_from_slice(&bytes);
    }
}

/// A place for the odd non-fatal note. Stands in for real logging, which the greeter
/// does not have wired up yet.
fn tracing_note(message: &str) {
    eprintln!("wlrix-greeter: {message}");
}

impl CompositorHandler for Greeter {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_factor: i32,
    ) {
        // Integer scale is assumed for now; fractional scale is a later concern.
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: wayland_client::protocol::wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        surface: &WlSurface,
        _time: u32,
    ) {
        // A frame callback came back. Redraw whichever surface it was for -- once the
        // greeter animates a caret this is where that happens; for now it just keeps
        // the surface valid across the compositor asking for a new frame.
        if surface == self.window.wl_surface() {
            self.draw_dialog(qh);
            return;
        }
        if let Some(index) = self
            .backdrops
            .iter()
            .position(|b| b.layer.wl_surface() == surface)
        {
            self.draw_backdrop(index, qh);
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &WlOutput,
    ) {
    }
}

impl LayerShellHandler for Greeter {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        // A backdrop's output went away; it is dropped when the output list is rebuilt.
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
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
        self.draw_backdrop(index, qh);
    }
}

impl WindowHandler for Greeter {
    fn request_close(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _window: &Window) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _window: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        // Honour a size the compositor insists on, but keep our own when it leaves the
        // choice to us (both dimensions zero), so the dialog stays the size it was
        // designed at.
        if let (Some(w), Some(h)) = configure.new_size {
            self.dialog_w = w.get();
            self.dialog_h = h.get();
        }
        self.draw_dialog(qh);
    }
}

impl OutputHandler for Greeter {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, _output: WlOutput) {
        self.rebuild_backdrops(qh);
    }

    fn update_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _output: WlOutput) {}

    fn output_destroyed(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, _output: WlOutput) {
        self.rebuild_backdrops(qh);
    }
}

impl SeatHandler for Greeter {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: WlSeat,
        _capability: Capability,
    ) {
        // Keyboard and pointer are wired up when input arrives, in a later stage.
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: WlSeat,
        _capability: Capability,
    ) {
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: WlSeat) {}
}

impl ShmHandler for Greeter {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
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
delegate_layer!(Greeter);
delegate_xdg_shell!(Greeter);
delegate_xdg_window!(Greeter);
delegate_registry!(Greeter);
