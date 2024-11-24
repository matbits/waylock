mod auth;
mod color;
mod config;
mod lock;
mod logger;
mod options;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle,
        },
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        keyboard::{KeyboardHandler, Keysym},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
        Capability, SeatHandler, SeatState,
    },
    session_lock::{
        SessionLock, SessionLockHandler, SessionLockState, SessionLockSurface,
        SessionLockSurfaceConfigure,
    },
    shm::{raw::RawPool, Shm, ShmHandler},
};
use std::process::Command;
use std::time::Duration;
use wayland_client::{
    globals::registry_queue_init,
    protocol::{
        wl_buffer, wl_keyboard, wl_output,
        wl_pointer::{self},
        wl_seat, wl_shm, wl_surface,
    },
    Connection, QueueHandle,
};

use crate::options::Options;

struct AppData {
    loop_handle: LoopHandle<'static, Self>,
    conn: Connection,
    compositor_state: CompositorState,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    output_state: OutputState,
    pointer: Option<wl_pointer::WlPointer>,
    registry_state: RegistryState,
    shm: Shm,
    seat_state: SeatState,
    session_lock_state: SessionLockState,
    session_lock: Option<SessionLock>,
    lock_surfaces: Vec<SessionLockSurface>,
    lock_surfaces_out: Vec<(SessionLockSurface, i32, i32)>,
    options: Options,
    lock_state: lock::LockState,
    color: u32,
    passwd: String,
    exit: bool,
    auth_hdl: auth::LockAuth,
}

fn main() {
    //env_logger::init();

    let conn = Connection::connect_to_env().expect("Error: ");
    let (globals, event_queue) = registry_queue_init(&conn).unwrap();
    let qh: QueueHandle<AppData> = event_queue.handle();
    let mut event_loop: EventLoop<AppData> =
        EventLoop::try_new().expect("Failed to initialize the event loop!");

    let qh: QueueHandle<AppData> = event_queue.handle();

    let mut app_data = AppData {
        loop_handle: event_loop.handle(),
        conn,
        compositor_state: CompositorState::bind(&globals, &qh).unwrap(),
        keyboard: None,
        pointer: None,
        output_state: OutputState::new(&globals, &qh),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        shm: Shm::bind(&globals, &qh).unwrap(),
        session_lock_state: SessionLockState::new(&globals, &qh),
        session_lock: None,
        lock_surfaces: Vec::new(),
        lock_surfaces_out: Vec::new(),
        options: Options::new(),
        lock_state: lock::LockState::Init,
        color: 0,
        passwd: String::new(),
        exit: false,
        auth_hdl: auth::LockAuth::new(),
    };

    app_data.color = app_data.options.init_color;

    app_data.session_lock =
        Some(app_data.session_lock_state.lock(&qh).expect("ext-session-lock not supported"));

    // After locking the session, we're expected to create a lock surface for each output.
    // As soon as all lock surfaces are created, `SessionLockHandler::locked` will be called
    // and the every surface receives a `SessionLockHandler::configure` call.
    for output in app_data.output_state().outputs() {
        let session_lock = app_data.session_lock.as_ref().unwrap();
        let surface = app_data.compositor_state.create_surface(&qh);

        // It's important to keep the `SessionLockSurface` returned here around, as the
        // surface will be destroyed when the `SessionLockSurface` is dropped.
        let lock_surface = session_lock.create_lock_surface(surface, &output, &qh);
        app_data.lock_surfaces.push(lock_surface);
    }

    WaylandSource::new(app_data.conn.clone(), event_queue).insert(event_loop.handle()).unwrap();

    loop {
        event_loop.dispatch(None, &mut app_data).unwrap();

        if app_data.exit {
            break;
        }
    }
}

impl SeatHandler for AppData {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            let keyboard = self
                .seat_state
                .get_keyboard_with_repeat(
                    qh,
                    &seat,
                    None,
                    self.loop_handle.clone(),
                    Box::new(|_state, _wl_kbd, _event| {
                        //println!("Repeat: {:?} ", event);
                    }),
                )
                .expect("Failed to create keyboard");

            self.keyboard = Some(keyboard);
        }

        if capability == Capability::Pointer && self.pointer.is_none() {
            println!("Set pointer capability");
            let pointer = self.seat_state.get_pointer(qh, &seat).expect("Failed to create pointer");
            self.pointer = Some(pointer);

            match &self.pointer {
                Some(pointer) => pointer.set_cursor(0, None, 0, 0),
                None => {}
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard && self.keyboard.is_some() {
            println!("Unset keyboard capability");
            self.keyboard.take().unwrap().release();
        }

        if capability == Capability::Pointer && self.pointer.is_some() {
            println!("Unset pointer capability");
            self.pointer.take().unwrap().release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl KeyboardHandler for AppData {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysym: &[smithay_client_toolkit::seat::keyboard::Keysym],
    ) {
        println!("enter keyboard")
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        println!("leave keyboard")
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _serial: u32,
        event: smithay_client_toolkit::seat::keyboard::KeyEvent,
    ) {
        let redraw = self.set_color(lock::LockState::Input);
        if redraw {
            self.redraw_all(qh);
        }

        match event.keysym {
            Keysym::KP_Enter | Keysym::Return => {
                let redraw = self.set_color(lock::LockState::Wait);
                if redraw {
                    self.redraw_all(qh);
                }

                if self.auth_hdl.check_password(self.passwd.as_str()) {
                    self.set_color(lock::LockState::Success);
                } else {
                    println!("failure!");
                    let redraw = self.set_color(lock::LockState::Fail);
                    if redraw {
                        self.redraw_all(qh);
                    }

                    self.loop_handle
                        .insert_source(
                            Timer::from_duration(Duration::from_secs(5)),
                            |_, _, app_data| {
                                // Unlock the lock
                                app_data.session_lock.take().unwrap().unlock();
                                // Sync connection to make sure compostor receives destroy
                                app_data.conn.roundtrip().unwrap();
                                // Then we can exit
                                app_data.exit = true;
                                TimeoutAction::Drop
                            },
                        )
                        .unwrap();
                }

                self.passwd.clear();
            }
            Keysym::Delete | Keysym::BackSpace => {
                self.passwd.pop();
            }
            Keysym::Escape => {
                self.passwd.clear();
            }
            _ => {
                if let Some(ch) = event.keysym.key_char() {
                    self.passwd.push(ch)
                }
            }
        }
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: smithay_client_toolkit::seat::keyboard::KeyEvent,
    ) {
        //println!("release!");
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wayland_client::protocol::wl_keyboard::WlKeyboard,
        _serial: u32,
        _modifiers: smithay_client_toolkit::seat::keyboard::Modifiers,
        _layout: u32,
    ) {
        //println!("update modifiers!")
    }
}

impl PointerHandler for AppData {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        //println!("pointer event");

        use PointerEventKind::*;
        for event in events {
            match event.kind {
                Enter { serial } => {
                    _pointer.set_cursor(serial, None, 0, 0);
                    println!("Pointer entered @{:?}", event.position);
                }
                Leave { .. } => {
                    println!("Pointer left");
                }
                Motion { .. } => {}
                Press { button, .. } => {
                    println!("Press {:x} @ {:?}", button, event.position);
                }
                Release { button, .. } => {
                    println!("Release {:x} @ {:?}", button, event.position);
                }
                Axis { horizontal, vertical, .. } => {
                    println!("Scroll H:{horizontal:?}, V:{vertical:?}");
                }
            }
        }
    }
}

impl SessionLockHandler for AppData {
    fn locked(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _session_lock: SessionLock) {
        println!("Locked");
    }

    fn finished(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _session_lock: SessionLock,
    ) {
        println!("Session could not be locked");
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        session_lock_surface: SessionLockSurface,
        configure: SessionLockSurfaceConfigure,
        _serial: u32,
    ) {
        println!("configure");
        let (width, height) = configure.new_size;
        self.redraw(qh, &session_lock_surface, width as i32, height as i32);
        self.lock_surfaces_out.push((session_lock_surface, width as i32, height as i32));
    }
}

impl CompositorHandler for AppData {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        println!("transform_changed");
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        println!("frame callback");
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // Not needed for this example.
        println!("surface entered")
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        // Not needed for this example.
    }
}

impl OutputHandler for AppData {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        println!("update:output");
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState,];
}

impl ShmHandler for AppData {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl AppData {
    pub fn set_color(&mut self, state: lock::LockState) -> bool {
        match self.lock_state {
            lock::LockState::Init => {
                if state != lock::LockState::Input {
                    return false;
                }

                self.lock_state = state;
                self.color = self.options.input_color;
            }
            lock::LockState::Input => {
                if state != lock::LockState::Wait {
                    return false;
                }

                self.lock_state = state;
                self.color = self.options.wait_color;
            }
            lock::LockState::Wait => {
                if state == lock::LockState::Fail {
                    self.lock_state = state;
                    self.color = self.options.fail_color;

                    if let Some(command) = &self.options.fail_command {
                        if let Err(err) = Command::new("sh").arg("-c").arg(command).spawn() {
                            log::warn!("Error executing fail command \"{}\": {}", command, err);
                        }
                    }

                    return true;
                }

                if state == lock::LockState::Success {
                    self.lock_state = state;

                    self.loop_handle
                        .insert_source(Timer::immediate(), |_, _, app_data| {
                            // Unlock the lock
                            app_data.session_lock.take().unwrap().unlock();
                            // Sync connection to make sure compostor receives destroy
                            app_data.conn.roundtrip().unwrap();
                            // Then we can exit
                            app_data.exit = true;
                            TimeoutAction::Drop
                        })
                        .unwrap();

                    return true;
                }

                return false;
            }
            lock::LockState::Fail => {
                if state != lock::LockState::Input {
                    return false;
                }

                self.lock_state = state;
                self.color = self.options.input_color;
            }
            lock::LockState::Success => {}
        }

        return true;
    }

    pub fn redraw(
        &self,
        qh: &QueueHandle<Self>,
        session_lock_surface: &SessionLockSurface,
        width: i32,
        height: i32,
    ) {
        let mut pool = RawPool::new(width as usize * height as usize * 4, &self.shm).unwrap();

        let buffer =
            pool.create_buffer(0, width, height, width * 4, wl_shm::Format::Argb8888, (), qh);

        // Write the current color to the buffer
        for (ptr, byte) in pool.mmap().iter_mut().zip(self.color.to_ne_bytes().iter().cycle()) {
            *ptr = *byte;
        }

        session_lock_surface.wl_surface().attach(Some(&buffer), 0, 0);
        session_lock_surface.wl_surface().damage_buffer(0, 0, width, height);
        session_lock_surface.wl_surface().commit();

        buffer.destroy();
    }

    pub fn redraw_all(&self) {
        for surface in self.lock_surfaces_out.iter() {
            self.redraw(qh, &surface.0, surface.1, surface.2);
        }
    }
}

smithay_client_toolkit::delegate_keyboard!(AppData);
smithay_client_toolkit::delegate_compositor!(AppData);
smithay_client_toolkit::delegate_pointer!(AppData);
smithay_client_toolkit::delegate_output!(AppData);
smithay_client_toolkit::delegate_session_lock!(AppData);
smithay_client_toolkit::delegate_shm!(AppData);
smithay_client_toolkit::delegate_seat!(AppData);
smithay_client_toolkit::delegate_registry!(AppData);
wayland_client::delegate_noop!(AppData: ignore wl_buffer::WlBuffer);
