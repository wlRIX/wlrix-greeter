// SPDX-License-Identifier: GPL-3.0-or-later
//! A one-window harness for looking at the dialog.
//!
//! The interesting part of the greeter -- the drawing -- can be seen without greetd,
//! without a login, without any of the state machinery. This opens a single toplevel of
//! the dialog's size and paints a [`dialog::Model`] into it, so `examples/paint_preview`
//! can put the whole login screen on screen from a hard-coded user list and it can be
//! screenshotted and compared against the reference.
//!
//! It is a development aid, not part of the login path.

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_registry, delegate_seat, delegate_shm,
    delegate_xdg_shell, delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{Capability, SeatHandler, SeatState},
    shell::{
        WaylandSurface,
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
    protocol::{wl_output::WlOutput, wl_seat::WlSeat, wl_shm, wl_surface::WlSurface},
};

use crate::theme::font::Fonts;
use crate::ui::canvas::Canvas;
use crate::ui::dialog::{self, Model};

/// Show `model` in a window of `width` x `height` until it is closed.
pub fn show(model: Model, width: u32, height: u32) -> Result<(), String> {
    let fonts = Fonts::load()?;

    let conn = Connection::connect_to_env()
        .map_err(|err| format!("no Wayland compositor to connect to: {err}"))?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|err| format!("could not read the registry: {err}"))?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|err| format!("wl_compositor unavailable: {err}"))?;
    let xdg_shell =
        XdgShell::bind(&globals, &qh).map_err(|err| format!("xdg-shell unavailable: {err}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|err| format!("wl_shm unavailable: {err}"))?;
    let pool = SlotPool::new((width * height * 4) as usize, &shm)
        .map_err(|err| format!("could not create a buffer pool: {err}"))?;

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestClient, &qh);
    window.set_title("wlRIX greeter preview");
    // The real app_id, so the compositor centers it exactly as it would the greeter.
    window.set_app_id("com.wlrix.greeter");
    window.set_min_size(Some((width, height)));
    window.set_max_size(Some((width, height)));
    window.commit();

    let mut preview = Preview {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        seat_state: SeatState::new(&globals, &qh),
        shm,
        pool,
        window,
        width,
        height,
        fonts,
        model,
        exit: false,
    };

    loop {
        event_queue
            .blocking_dispatch(&mut preview)
            .map_err(|err| format!("event loop failed: {err}"))?;
        if preview.exit {
            return Ok(());
        }
    }
}

struct Preview {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    width: u32,
    height: u32,
    fonts: Fonts,
    model: Model,
    exit: bool,
}

impl Preview {
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        let (width, height) = (self.width.max(1), self.height.max(1));
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
        dialog::paint(&mut canvas, &mut self.fonts, &self.model);

        let surface = self.window.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        if let Err(err) = buffer.attach_to(surface) {
            eprintln!("paint_preview: could not attach buffer: {err}");
        }
        surface.frame(qh, surface.clone());
        surface.commit();
    }
}

impl CompositorHandler for Preview {
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
    fn frame(&mut self, _c: &Connection, qh: &QueueHandle<Self>, _s: &WlSurface, _t: u32) {
        self.draw(qh);
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

impl WindowHandler for Preview {
    fn request_close(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _w: &Window) {
        self.exit = true;
    }
    fn configure(
        &mut self,
        _c: &Connection,
        qh: &QueueHandle<Self>,
        _w: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        if let (Some(w), Some(h)) = configure.new_size {
            self.width = w.get();
            self.height = h.get();
        }
        self.draw(qh);
    }
}

impl OutputHandler for Preview {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: WlOutput) {}
    fn update_output(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: WlOutput) {}
    fn output_destroyed(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _o: WlOutput) {}
}

impl SeatHandler for Preview {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _s: WlSeat) {}
    fn new_capability(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: WlSeat,
        _cap: Capability,
    ) {
    }
    fn remove_capability(
        &mut self,
        _c: &Connection,
        _q: &QueueHandle<Self>,
        _s: WlSeat,
        _cap: Capability,
    ) {
    }
    fn remove_seat(&mut self, _c: &Connection, _q: &QueueHandle<Self>, _s: WlSeat) {}
}

impl ShmHandler for Preview {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Preview {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_compositor!(Preview);
delegate_output!(Preview);
delegate_shm!(Preview);
delegate_seat!(Preview);
delegate_xdg_shell!(Preview);
delegate_xdg_window!(Preview);
delegate_registry!(Preview);
