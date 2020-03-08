mod auth;
mod env;
mod input;
mod output;
mod surface;

use crate::options::Options;

use self::auth::LockAuth;
use self::env::LockEnv;
use self::input::LockInput;
use self::output::OutputHandling;
use self::surface::LockSurface;

use smithay_client_toolkit::{
    reexports::{
        calloop,
        client::protocol::{wl_compositor, wl_shm},
        protocols::wlr::unstable::input_inhibitor::v1::client::zwlr_input_inhibit_manager_v1,
        protocols::wlr::unstable::layer_shell::v1::client::zwlr_layer_shell_v1,
    },
    seat::keyboard::keysyms,
    WaylandSource,
};

use std::{cell::RefCell, rc::Rc};

pub fn lock_screen(options: &Options) -> std::io::Result<()> {
    let (lock_env, display, queue) = LockEnv::init_environment()?;

    let _inhibitor = lock_env
        .require_global::<zwlr_input_inhibit_manager_v1::ZwlrInputInhibitManagerV1>()
        .get_inhibitor();

    let lock_surfaces = {
        let compositor = lock_env.require_global::<wl_compositor::WlCompositor>();
        let layer_shell = lock_env.require_global::<zwlr_layer_shell_v1::ZwlrLayerShellV1>();
        let shm = lock_env.require_global::<wl_shm::WlShm>();
        let color = options.color;

        let lock_surfaces = Rc::new(RefCell::new(Vec::new()));

        let lock_surfaces_handle = Rc::clone(&lock_surfaces);
        lock_env.set_output_created_listener(Some(move |id, output| {
            (*lock_surfaces_handle.borrow_mut()).push((
                id,
                LockSurface::new(
                    &output,
                    compositor.clone(),
                    layer_shell.clone(),
                    shm.clone(),
                    color,
                ),
            ));
        }));

        let lock_surfaces_handle = Rc::clone(&lock_surfaces);
        lock_env.set_output_removed_listener(Some(move |id| {
            lock_surfaces_handle.borrow_mut().retain(|(i, _)| *i != id);
        }));

        lock_surfaces
    };

    let mut event_loop = calloop::EventLoop::<()>::new()?;

    let lock_input = LockInput::new(&lock_env, event_loop.handle())?;

    let _source_queue =
        event_loop
            .handle()
            .insert_source(WaylandSource::new(queue), |ret, _| {
                if let Err(e) = ret {
                    panic!("Wayland connection lost: {:?}", e);
                }
            })?;

    let lock_auth = LockAuth::new();
    let mut current_password = String::new();

    loop {
        // Handle all input recieved since last check
        while let Some((keysym, utf8)) = lock_input.pop() {
            match keysym {
                keysyms::XKB_KEY_KP_Enter | keysyms::XKB_KEY_Return => {
                    if lock_auth.check_password(&current_password) {
                        return Ok(());
                    } else {
                        for (_, lock_surface) in lock_surfaces.borrow_mut().iter_mut() {
                            lock_surface.set_color(options.fail_color);
                        }
                    }
                }
                keysyms::XKB_KEY_Delete | keysyms::XKB_KEY_BackSpace => {
                    current_password.pop();
                }
                keysyms::XKB_KEY_Escape => {
                    current_password.clear();
                }
                _ => {
                    if let Some(new_input) = utf8 {
                        current_password.push_str(&new_input);
                    }
                }
            }
        }

        // This is ugly, let's hope that some version of drain_filter() gets stablized soon
        // https://github.com/rust-lang/rust/issues/43244
        {
            let mut lock_surfaces = lock_surfaces.borrow_mut();
            let mut i = 0;
            while i != lock_surfaces.len() {
                if lock_surfaces[i].1.handle_events() {
                    lock_surfaces.remove(i);
                } else {
                    i += 1;
                }
            }
        }

        display.flush()?;
        event_loop.dispatch(None, &mut ())?;
    }
}