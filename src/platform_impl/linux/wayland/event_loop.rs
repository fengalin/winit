use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    fmt,
    rc::Rc,
    sync::{mpsc::SendError, Arc, Mutex},
    time::{Duration, Instant},
};

use calloop;

use smithay_client_toolkit::reexports::protocols::unstable::pointer_constraints::v1::client::{
    zwp_locked_pointer_v1::ZwpLockedPointerV1, zwp_pointer_constraints_v1::ZwpPointerConstraintsV1,
};
use smithay_client_toolkit::reexports::protocols::unstable::relative_pointer::v1::client::{
    zwp_relative_pointer_manager_v1::ZwpRelativePointerManagerV1,
    zwp_relative_pointer_v1::ZwpRelativePointerV1,
};

use smithay_client_toolkit::{
    default_environment,
    environment::Environment,
    init_default_environment,
    output::with_output_info,
    reexports::client::{
        protocol::{wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface, wl_touch},
        Attached, ConnectError, Display, EventQueue,
    },
    seat::{
        self,
        // FIXME pointer::{AutoPointer, AutoThemer},
    },
    window::Decorations,
};

use crate::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event::{
        DeviceEvent, DeviceId as RootDeviceId, Event, ModifiersState, StartCause, WindowEvent,
    },
    event_loop::{ControlFlow, EventLoopClosed, EventLoopWindowTarget as RootELW},
    monitor::{MonitorHandle as RootMonitorHandle, VideoMode as RootVideoMode},
    platform_impl::platform::{
        sticky_exit_callback, DeviceId as PlatformDeviceId, MonitorHandle as PlatformMonitorHandle,
        VideoMode as PlatformVideoMode, WindowId as PlatformWindowId,
    },
    window::{CursorIcon, WindowId as RootWindowId},
};

use super::{
    window::{DecorationsAction, WindowStore},
    DeviceId, WindowId,
};

default_environment!(DefaultEnvironment, desktop);

#[derive(Clone)]
pub struct EventsSink {
    sender: calloop::channel::Sender<Event<'static, ()>>,
}

impl EventsSink {
    pub fn new(sender: calloop::channel::Sender<Event<'static, ()>>) -> EventsSink {
        EventsSink { sender }
    }

    pub fn send_event(&self, event: Event<'static, ()>) {
        self.sender.send(event).unwrap()
    }

    pub fn send_device_event(&self, event: DeviceEvent, device_id: DeviceId) {
        self.send_event(Event::DeviceEvent {
            event,
            device_id: RootDeviceId(PlatformDeviceId::Wayland(device_id)),
        });
    }

    pub fn send_window_event(&self, event: WindowEvent<'static>, window_id: WindowId) {
        self.send_event(Event::WindowEvent {
            event,
            window_id: RootWindowId(PlatformWindowId::Wayland(window_id)),
        });
    }
}

pub struct CursorManager {
    pointer_constraints_proxy: Arc<Mutex<Option<ZwpPointerConstraintsV1>>>,
    // FIXME auto_themer: Option<AutoThemer>,
    // FIXME pointers: Vec<AutoPointer>,
    locked_pointers: Vec<ZwpLockedPointerV1>,
    cursor_visible: bool,
    current_cursor: CursorIcon,
    scale_factor: u32,
}

impl CursorManager {
    fn new(constraints: Arc<Mutex<Option<ZwpPointerConstraintsV1>>>) -> CursorManager {
        CursorManager {
            pointer_constraints_proxy: constraints,
            // FIXME auto_themer: None,
            // FIXME pointers: Vec::new(),
            locked_pointers: Vec::new(),
            cursor_visible: true,
            current_cursor: CursorIcon::default(),
            scale_factor: 1,
        }
    }

    // FIXME
    /*
    fn register_pointer(&mut self, pointer: wl_pointer::WlPointer) {
        let auto_themer = self
            .auto_themer
            .as_ref()
            .expect("AutoThemer not initialized. Server did not advertise shm or compositor?");
        self.pointers.push(auto_themer.theme_pointer(pointer));
    }

    fn set_auto_themer(&mut self, auto_themer: AutoThemer) {
        self.auto_themer = Some(auto_themer);
    }
    */

    pub fn set_cursor_visible(&mut self, visible: bool) {
        if !visible {
            // FIXME
            /*
            for pointer in self.pointers.iter() {
                (**pointer).set_cursor(0, None, 0, 0);
            }
            */
        } else {
            self.set_cursor_icon_impl(self.current_cursor);
        }
        self.cursor_visible = visible;
    }

    /// A helper function to restore cursor styles on PtrEvent::Enter.
    pub fn reload_cursor_style(&mut self) {
        if !self.cursor_visible {
            self.set_cursor_visible(false);
        } else {
            self.set_cursor_icon_impl(self.current_cursor);
        }
    }

    pub fn set_cursor_icon(&mut self, cursor: CursorIcon) {
        if cursor != self.current_cursor {
            self.current_cursor = cursor;
            if self.cursor_visible {
                self.set_cursor_icon_impl(cursor);
            }
        }
    }

    pub fn update_scale_factor(&mut self, scale: u32) {
        self.scale_factor = scale;
        self.reload_cursor_style();
    }

    fn set_cursor_icon_impl(&mut self, cursor: CursorIcon) {
        let cursor = match cursor {
            CursorIcon::Alias => "link",
            CursorIcon::Arrow => "arrow",
            CursorIcon::Cell => "plus",
            CursorIcon::Copy => "copy",
            CursorIcon::Crosshair => "crosshair",
            CursorIcon::Default => "left_ptr",
            CursorIcon::Hand => "hand",
            CursorIcon::Help => "question_arrow",
            CursorIcon::Move => "move",
            CursorIcon::Grab => "grab",
            CursorIcon::Grabbing => "grabbing",
            CursorIcon::Progress => "progress",
            CursorIcon::AllScroll => "all-scroll",
            CursorIcon::ContextMenu => "context-menu",

            CursorIcon::NoDrop => "no-drop",
            CursorIcon::NotAllowed => "crossed_circle",

            // Resize cursors
            CursorIcon::EResize => "right_side",
            CursorIcon::NResize => "top_side",
            CursorIcon::NeResize => "top_right_corner",
            CursorIcon::NwResize => "top_left_corner",
            CursorIcon::SResize => "bottom_side",
            CursorIcon::SeResize => "bottom_right_corner",
            CursorIcon::SwResize => "bottom_left_corner",
            CursorIcon::WResize => "left_side",
            CursorIcon::EwResize => "h_double_arrow",
            CursorIcon::NsResize => "v_double_arrow",
            CursorIcon::NwseResize => "bd_double_arrow",
            CursorIcon::NeswResize => "fd_double_arrow",
            CursorIcon::ColResize => "h_double_arrow",
            CursorIcon::RowResize => "v_double_arrow",

            CursorIcon::Text => "text",
            CursorIcon::VerticalText => "vertical-text",

            CursorIcon::Wait => "watch",

            CursorIcon::ZoomIn => "zoom-in",
            CursorIcon::ZoomOut => "zoom-out",
        };

        // FIXME
        /*
        for pointer in self.pointers.iter() {
            // Ignore erros, since we don't want to fail hard in case we can't find a proper cursor
            // in a given theme.
            let _ = pointer.set_cursor(cursor, None);
        }
        */
    }

    // This function can only be called from a thread on which `pointer_constraints_proxy` event
    // queue is located, so calling it directly from a Window doesn't work well, in case
    // you've sent your window to another thread, so we need to pass cursor grab updates to
    // the event loop and call this function from there.
    fn grab_pointer(&mut self, surface: Option<&wl_surface::WlSurface>) {
        for locked_pointer in self.locked_pointers.drain(..) {
            locked_pointer.destroy();
        }

        if let Some(surface) = surface {
            // FIXME
            /*
            for pointer in self.pointers.iter() {
                let locked_pointer = self
                    .pointer_constraints_proxy
                    .try_lock()
                    .unwrap()
                    .as_ref()
                    .map(|pointer_constraints| {
                        super::pointer::implement_locked_pointer(
                            surface,
                            &**pointer,
                            pointer_constraints,
                        )
                    });

                if let Some(locked_pointer) = locked_pointer {
                    self.locked_pointers.push(locked_pointer);
                }
            }
            */
        }
    }
}

pub struct EventLoop<T: 'static> {
    // The wayland display
    pub display: Arc<Display>,
    pub outputs: Vec<wl_output::WlOutput>,
    // The cursor manager
    cursor_manager: Arc<Mutex<CursorManager>>,
    kbd_channel: Option<calloop::channel::Channel<Event<'static, ()>>>,
    user_channel: Option<calloop::channel::Channel<T>>,
    user_sender: calloop::channel::Sender<T>,
    window_target: Option<RootELW<T>>,
    // Hold the seat listener so as to keep it alive
    _seat_listener: seat::SeatListener,
}

// A handle that can be sent across threads and used to wake up the `EventLoop`.
//
// We should only try and wake up the `EventLoop` if it still exists, so we hold Weak ptrs.
pub struct EventLoopProxy<T: 'static> {
    user_sender: calloop::channel::Sender<T>,
}

pub struct EventLoopWindowTarget<T> {
    // the event queue
    pub evq: RefCell<EventQueue>,
    // The window store
    pub store: Arc<Mutex<WindowStore>>,
    // The cursor manager
    pub cursor_manager: Arc<Mutex<CursorManager>>,
    // The env
    pub env: Environment<DefaultEnvironment>,
    // A cleanup switch to prune dead windows
    pub cleanup_needed: Arc<Mutex<bool>>,
    // The wayland display
    pub display: Arc<Display>,
    // FIXME this seems wrong + why would we keep _marker?
    _marker: ::std::marker::PhantomData<T>,
}

impl<T: 'static> Clone for EventLoopProxy<T> {
    fn clone(&self) -> Self {
        EventLoopProxy {
            user_sender: self.user_sender.clone(),
        }
    }
}

impl<T: 'static> EventLoopProxy<T> {
    pub fn send_event(&self, event: T) -> Result<(), EventLoopClosed<T>> {
        self.user_sender.send(event).map_err(|e| {
            let SendError(x) = e;
            EventLoopClosed(x)
        })
    }
}

impl<T: 'static> EventLoop<T> {
    pub fn new() -> Result<EventLoop<T>, ConnectError> {
        let (env, display, event_queue) = init_default_environment!(DefaultEnvironment, desktop)?;

        let display = Arc::new(display);
        let store = Arc::new(Mutex::new(WindowStore::new()));

        let (kbd_sender, kbd_channel) = calloop::channel::channel();
        let sink = EventsSink::new(kbd_sender);

        let pointer_constraints_proxy = Arc::new(Mutex::new(None));

        let mut seat_manager = SeatManager {
            sink,
            store: store.clone(),
            seats: Arc::new(Mutex::new(HashMap::new())),
            relative_pointer_manager_proxy: Rc::new(RefCell::new(None)),
            pointer_constraints_proxy: pointer_constraints_proxy.clone(),
            cursor_manager: Arc::new(Mutex::new(CursorManager::new(pointer_constraints_proxy))),
        };

        // FIXME cursor_manager used to be called in env callback to set set_auto_themer(auto_themer);
        // note that there is a ThemeManager & AutoThemer classes in smithay...tk
        // check if this is handled automagically by the default env init
        let cursor_manager = seat_manager.cursor_manager.clone();

        for seat in env.get_all_seats().iter() {
            let seat_clone = seat.clone();
            seat::with_seat_data(&seat, |sct_seat_data| {
                if !sct_seat_data.defunct {
                    seat_manager.add_seat(seat_clone, sct_seat_data);
                }
            });
        }

        // FIXME keep the listner alive
        let seat_listener = env.listen_for_seats(move |seat, sct_seat_data, _| {
            if !sct_seat_data.defunct {
                seat_manager.add_seat(seat, sct_seat_data);
            } else {
                seat_manager.remove_seat(&sct_seat_data.name);
            }
        });

        // FIXME the seat_manager was also used for relative_pointer & pointer_constraints stuff
        // in the env callback

        let (user_sender, user_channel) = calloop::channel::channel();

        Ok(EventLoop {
            display: display.clone(),
            outputs: env.get_all_outputs(),
            user_sender,
            user_channel: Some(user_channel),
            kbd_channel: Some(kbd_channel),
            cursor_manager: cursor_manager.clone(),
            window_target: Some(RootELW {
                p: crate::platform_impl::EventLoopWindowTarget::Wayland(EventLoopWindowTarget {
                    evq: RefCell::new(event_queue),
                    store,
                    env,
                    cursor_manager,
                    cleanup_needed: Arc::new(Mutex::new(false)),
                    display,
                    _marker: ::std::marker::PhantomData,
                }),
                _marker: ::std::marker::PhantomData,
            }),
            _seat_listener: seat_listener,
        })
    }

    pub fn create_proxy(&self) -> EventLoopProxy<T> {
        EventLoopProxy {
            user_sender: self.user_sender.clone(),
        }
    }

    pub fn run<F>(mut self, callback: F) -> !
    where
        F: 'static + FnMut(Event<'_, T>, &RootELW<T>, &mut ControlFlow),
    {
        self.run_return(callback);
        std::process::exit(0);
    }

    pub fn run_return<F>(&mut self, callback: F)
    where
        F: FnMut(Event<'_, T>, &RootELW<T>, &mut ControlFlow),
    {
        struct Data<T: 'static, F> {
            window_target: RootELW<T>,
            control_flow: ControlFlow,
            callback: F,
        }

        let mut data = Data {
            window_target: self.window_target.take().unwrap(),
            control_flow: ControlFlow::default(),
            callback,
        };

        // send pending events to the server
        self.display.flush().expect("Wayland connection lost.");

        let mut event_loop =
            calloop::EventLoop::new().expect("Failed to initialize the event loop!");
        let loop_handle = event_loop.handle();

        // FIXME add evq too

        let kbd_dispatcher = calloop::Dispatcher::new(
            self.kbd_channel.take().unwrap(),
            move |event, _, data: &mut Data<_, F>| {
                if let calloop::channel::Event::Msg(event) = event {
                    let event = event.map_nonuser_event().unwrap();
                    sticky_exit_callback(
                        event,
                        &data.window_target,
                        &mut data.control_flow,
                        &mut data.callback,
                    );
                }
            },
        );
        let kbd_token = loop_handle
            .register_dispatcher(kbd_dispatcher.clone())
            .unwrap();

        let user_channel_dispatcher = calloop::Dispatcher::new(
            self.user_channel.take().unwrap(),
            move |event, _, data: &mut Data<_, F>| {
                if let calloop::channel::Event::Msg(event) = event {
                    sticky_exit_callback(
                        Event::UserEvent(event),
                        &data.window_target,
                        &mut data.control_flow,
                        &mut data.callback,
                    );
                }
            },
        );
        let user_token = loop_handle
            .register_dispatcher(user_channel_dispatcher.clone())
            .unwrap();

        (data.callback)(
            Event::NewEvents(StartCause::Init),
            &data.window_target,
            &mut data.control_flow,
        );

        loop {
            // Read events from the event queue
            event_loop.dispatch(None, &mut data).unwrap();

            // FIXME there an idle source than may be applicable for those

            // This is where most of the window events are triggered
            // (except for redraw - see below)
            self.post_dispatch_triggers(
                &data.window_target,
                &mut data.callback,
                &mut data.control_flow,
            );

            // send Events cleared
            sticky_exit_callback(
                Event::MainEventsCleared,
                &data.window_target,
                &mut data.control_flow,
                &mut data.callback,
            );

            // handle request-redraw
            {
                let Data {
                    window_target,
                    control_flow,
                    callback,
                } = &mut data;

                self.redraw_triggers(window_target, |wid, window_target| {
                    sticky_exit_callback(
                        Event::RedrawRequested(crate::window::WindowId(
                            crate::platform_impl::WindowId::Wayland(wid),
                        )),
                        window_target,
                        control_flow,
                        callback,
                    );
                });
            }

            // send RedrawEventsCleared
            sticky_exit_callback(
                Event::RedrawEventsCleared,
                &data.window_target,
                &mut data.control_flow,
                &mut data.callback,
            );

            // send pending events to the server
            self.display.flush().expect("Wayland connection lost.");

            // During the run of the user callback, some other code monitoring and reading the
            // wayland socket may have been run (mesa for example does this with vsync), if that
            // is the case, some events may have been enqueued in our event queue.
            //
            // If some messages are there, the event loop needs to behave as if it was instantly
            // woken up by messages arriving from the wayland socket, to avoid getting stuck.
            let instant_wakeup = {
                let window_target = match data.window_target.p {
                    crate::platform_impl::EventLoopWindowTarget::Wayland(ref wt) => wt,
                    _ => unreachable!(),
                };
                let dispatched = window_target
                    .evq
                    .borrow_mut()
                    // FIXME args...
                    .dispatch_pending(&mut (), |_, _, _| ())
                    .expect("Wayland connection lost.");
                dispatched > 0
            };

            match data.control_flow {
                ControlFlow::Exit => break,
                ControlFlow::Poll => {
                    // non-blocking dispatch
                    event_loop
                        .dispatch(Duration::from_millis(0), &mut data)
                        .unwrap();

                    (data.callback)(
                        Event::NewEvents(StartCause::Poll),
                        &data.window_target,
                        &mut data.control_flow,
                    );
                }
                ControlFlow::Wait => {
                    if !instant_wakeup {
                        event_loop.dispatch(None, &mut data).unwrap();
                    }

                    (data.callback)(
                        Event::NewEvents(StartCause::WaitCancelled {
                            start: Instant::now(),
                            requested_resume: None,
                        }),
                        &data.window_target,
                        &mut data.control_flow,
                    );
                }
                ControlFlow::WaitUntil(deadline) => {
                    let start = Instant::now();
                    // compute the blocking duration
                    let duration = if deadline > start && !instant_wakeup {
                        deadline - start
                    } else {
                        Duration::from_millis(0)
                    };
                    event_loop.dispatch(Some(duration), &mut data).unwrap();

                    let now = Instant::now();
                    if now < deadline {
                        (data.callback)(
                            Event::NewEvents(StartCause::WaitCancelled {
                                start,
                                requested_resume: Some(deadline),
                            }),
                            &data.window_target,
                            &mut data.control_flow,
                        );
                    } else {
                        (data.callback)(
                            Event::NewEvents(StartCause::ResumeTimeReached {
                                start,
                                requested_resume: deadline,
                            }),
                            &data.window_target,
                            &mut data.control_flow,
                        );
                    }
                }
            }
        }

        (data.callback)(
            Event::LoopDestroyed,
            &data.window_target,
            &mut data.control_flow,
        );

        // get the channels back in
        loop_handle.remove(kbd_token);
        self.kbd_channel = Some(kbd_dispatcher.into_source_inner());
        loop_handle.remove(user_token);
        self.user_channel = Some(user_channel_dispatcher.into_source_inner());

        self.window_target = Some(data.window_target);
    }

    pub fn primary_monitor(&self) -> MonitorHandle {
        primary_monitor(&self.outputs)
    }

    pub fn available_monitors(&self) -> VecDeque<MonitorHandle> {
        available_monitors(&self.outputs)
    }

    pub fn window_target(&self) -> &RootELW<T> {
        self.window_target.as_ref().unwrap()
    }
}

impl<T> EventLoopWindowTarget<T> {
    pub fn display(&self) -> &Display {
        &*self.display
    }
}

/*
 * Private EventLoop Internals
 */

impl<T> EventLoop<T> {
    fn redraw_triggers<F>(&mut self, window_target: &RootELW<T>, mut callback: F)
    where
        F: FnMut(WindowId, &RootELW<T>),
    {
        let window_target_w = match window_target.p {
            crate::platform_impl::EventLoopWindowTarget::Wayland(ref wt) => wt,
            _ => unreachable!(),
        };
        window_target_w
            .store
            .lock()
            .unwrap()
            .for_each_redraw_trigger(|refresh, frame_refresh, wid, frame| {
                if let Some(frame) = frame {
                    if frame_refresh {
                        frame.refresh();
                        if !refresh {
                            frame.surface().commit()
                        }
                    }
                }
                if refresh {
                    callback(wid, window_target);
                }
            })
    }

    fn post_dispatch_triggers<F>(
        &mut self,
        window_target: &RootELW<T>,
        callback: &mut F,
        control_flow: &mut ControlFlow,
    ) where
        F: FnMut(Event<'_, T>, &RootELW<T>, &mut ControlFlow),
    {
        let window_target_w = match window_target.p {
            crate::platform_impl::EventLoopWindowTarget::Wayland(ref wt) => wt,
            _ => unreachable!(),
        };

        let mut callback = |event: Event<'_, T>| {
            sticky_exit_callback(event, window_target, control_flow, callback);
        };

        // prune possible dead windows
        {
            let mut cleanup_needed = window_target_w.cleanup_needed.lock().unwrap();
            if *cleanup_needed {
                let pruned = window_target_w.store.lock().unwrap().cleanup();
                *cleanup_needed = false;
                for wid in pruned {
                    callback(Event::WindowEvent {
                        window_id: crate::window::WindowId(
                            crate::platform_impl::WindowId::Wayland(wid),
                        ),
                        event: WindowEvent::Destroyed,
                    });
                }
            }
        }
        // process pending resize/refresh
        window_target_w.store.lock().unwrap().for_each(|window| {
            let window_id =
                crate::window::WindowId(crate::platform_impl::WindowId::Wayland(window.wid));

            // Update window logical .size field (for callbacks using .inner_size)
            let (old_logical_size, mut logical_size) = {
                let mut window_size = window.size.lock().unwrap();
                let old_logical_size = *window_size;
                *window_size = window.new_size.unwrap_or(old_logical_size);
                (old_logical_size, *window_size)
            };

            if let Some(scale_factor) = window.new_scale_factor {
                // Update cursor scale factor
                self.cursor_manager
                    .lock()
                    .unwrap()
                    .update_scale_factor(scale_factor as u32);
                let new_logical_size = {
                    let scale_factor = scale_factor as f64;
                    let mut physical_size =
                        LogicalSize::<f64>::from(logical_size).to_physical(scale_factor);
                    callback(Event::WindowEvent {
                        window_id,
                        event: WindowEvent::ScaleFactorChanged {
                            scale_factor,
                            new_inner_size: &mut physical_size,
                        },
                    });
                    physical_size.to_logical::<u32>(scale_factor).into()
                };
                // Update size if changed by callback
                if new_logical_size != logical_size {
                    logical_size = new_logical_size;
                    *window.size.lock().unwrap() = logical_size.into();
                }
            }

            if window.new_size.is_some() || window.new_scale_factor.is_some() {
                if let Some(frame) = window.frame {
                    // Update decorations state
                    match window.decorations_action {
                        Some(DecorationsAction::Hide) => frame.set_decorate(Decorations::None),
                        Some(DecorationsAction::Show) => {
                            frame.set_decorate(Decorations::FollowServer)
                        }
                        None => (),
                    }

                    // mutter (GNOME Wayland) relies on `set_geometry` to reposition window in case
                    // it overlaps mutter's `bounding box`, so we can't avoid this resize call,
                    // which calls `set_geometry` under the hood, for now.
                    let (w, h) = logical_size;
                    frame.resize(w, h);
                    frame.refresh();
                }
                // Don't send resize event downstream if the new logical size and scale is identical to the
                // current one
                if logical_size != old_logical_size || window.new_scale_factor.is_some() {
                    let physical_size = LogicalSize::<f64>::from(logical_size).to_physical(
                        window.new_scale_factor.unwrap_or(window.prev_scale_factor) as f64,
                    );
                    callback(Event::WindowEvent {
                        window_id,
                        event: WindowEvent::Resized(physical_size),
                    });
                }
            }

            if window.closed {
                callback(Event::WindowEvent {
                    window_id,
                    event: WindowEvent::CloseRequested,
                });
            }

            if let Some(grab_cursor) = window.grab_cursor {
                let surface = if grab_cursor {
                    Some(window.surface)
                } else {
                    None
                };
                self.cursor_manager.lock().unwrap().grab_pointer(surface);
            }
        })
    }
}

fn get_target<T>(target: &RootELW<T>) -> &EventLoopWindowTarget<T> {
    match target.p {
        crate::platform_impl::EventLoopWindowTarget::Wayland(ref wt) => wt,
        _ => unreachable!(),
    }
}

/*
 * Wayland protocol implementations
 */

struct SeatManager {
    sink: EventsSink,
    store: Arc<Mutex<WindowStore>>,
    seats: Arc<Mutex<HashMap<String, SeatData>>>,
    relative_pointer_manager_proxy: Rc<RefCell<Option<ZwpRelativePointerManagerV1>>>,
    pointer_constraints_proxy: Arc<Mutex<Option<ZwpPointerConstraintsV1>>>,
    cursor_manager: Arc<Mutex<CursorManager>>,
}

impl SeatManager {
    // FIXME this doesn't allow handling an update to the seat: what happens if the keyboard is added
    // afterwards?
    // We need some sort of an update seat which monitors defunct for and act accordingly
    fn add_seat(&mut self, seat: Attached<wl_seat::WlSeat>, sct_seat_data: &seat::SeatData) {
        let mut seats = self.seats.lock().unwrap();

        let seat_data = SeatData::new(
            seat.clone(),
            sct_seat_data,
            self.sink.clone(),
            self.store.clone(),
            self.relative_pointer_manager_proxy.clone(),
            self.cursor_manager.clone(),
        );

        assert!(seats
            .insert(sct_seat_data.name.clone(), seat_data)
            .is_none());
    }

    fn remove_seat(&mut self, name: &str) {
        let _ = self.seats.lock().unwrap().remove(name);
    }
}

struct SeatData {
    seat: Attached<wl_seat::WlSeat>,
    sink: EventsSink,
    // FIXME store could be used as DispatchData
    store: Arc<Mutex<WindowStore>>,
    pointer: Option<wl_pointer::WlPointer>,
    // FIXME could we get rid of those 2?
    relative_pointer: Option<ZwpRelativePointerV1>,
    relative_pointer_manager_proxy: Rc<RefCell<Option<ZwpRelativePointerManagerV1>>>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    touch: Option<wl_touch::WlTouch>,
    // FIXME why do we need Arc<Mutex<>> for those?
    modifiers_tracker: Arc<Mutex<ModifiersState>>,
    cursor_manager: Arc<Mutex<CursorManager>>,
}

impl SeatData {
    // FIXME add an update method to monitor changes
    fn new(
        seat: Attached<wl_seat::WlSeat>,
        sct_seat_data: &seat::SeatData,
        sink: EventsSink,
        store: Arc<Mutex<WindowStore>>,
        relative_pointer_manager_proxy: Rc<RefCell<Option<ZwpRelativePointerManagerV1>>>,
        cursor_manager: Arc<Mutex<CursorManager>>,
    ) -> Self {
        let mut this = SeatData {
            seat,
            sink,
            store,
            pointer: None,
            relative_pointer: None,
            relative_pointer_manager_proxy,
            keyboard: None,
            touch: None,
            modifiers_tracker: Arc::new(Mutex::new(ModifiersState::default())),
            cursor_manager,
        };

        this.receive(sct_seat_data);

        this
    }

    fn receive(&mut self, sct_seat_data: &seat::SeatData) {
        assert!(!sct_seat_data.defunct);

        if !sct_seat_data.has_pointer {
            if let Some(pointer) = self.pointer.take() {
                if pointer.as_ref().version() >= 3 {
                    pointer.release();
                }
            }
        } else if self.pointer.is_none() {
            self.pointer = Some(super::pointer::implement_pointer(
                &self.seat,
                self.sink.clone(),
                self.store.clone(),
                self.modifiers_tracker.clone(),
                self.cursor_manager.clone(),
            ));

            // FIXME
            /*
            self.cursor_manager
                .lock()
                .unwrap()
                // FIXME is this the appropriate way to pass the pointer?
                .register_pointer(self.pointer.as_ref().unwrap().clone());
            */

            self.relative_pointer = self
                .relative_pointer_manager_proxy
                .try_borrow()
                .unwrap()
                .as_ref()
                .map(|manager| {
                    super::pointer::implement_relative_pointer(
                        self.sink.clone(),
                        // FIXME is this the appropriate way to pass the pointer?
                        self.pointer.as_ref().unwrap(),
                        manager,
                    )
                })
        } // else pointer already assigned

        if !sct_seat_data.has_keyboard {
            if let Some(kbd) = self.keyboard.take() {
                if kbd.as_ref().version() >= 3 {
                    kbd.release();
                }
            }
        } else if self.keyboard.is_none() {
            self.keyboard = Some(super::keyboard::init_keyboard(
                &self.seat,
                self.sink.clone(),
                Arc::clone(&self.modifiers_tracker),
            ));
        } // else keyboard already assigned

        if !sct_seat_data.has_touch {
            if let Some(touch) = self.touch.take() {
                if touch.as_ref().version() >= 3 {
                    touch.release();
                }
            }
        } else if self.touch.is_none() {
            self.touch = Some(super::touch::implement_touch(
                &self.seat,
                self.sink.clone(),
                self.store.clone(),
            ));
        } // else touch already assigned
    }
}

impl Drop for SeatData {
    fn drop(&mut self) {
        if let Some(pointer) = self.pointer.take() {
            if pointer.as_ref().version() >= 3 {
                pointer.release();
            }
        }
        if let Some(kbd) = self.keyboard.take() {
            if kbd.as_ref().version() >= 3 {
                kbd.release();
            }
        }
        if let Some(touch) = self.touch.take() {
            if touch.as_ref().version() >= 3 {
                touch.release();
            }
        }
    }
}

/*
 * Monitor stuff
 */

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VideoMode {
    pub(crate) size: (u32, u32),
    pub(crate) bit_depth: u16,
    pub(crate) refresh_rate: u16,
    pub(crate) monitor: MonitorHandle,
}

impl VideoMode {
    #[inline]
    pub fn size(&self) -> PhysicalSize<u32> {
        self.size.into()
    }

    #[inline]
    pub fn bit_depth(&self) -> u16 {
        self.bit_depth
    }

    #[inline]
    pub fn refresh_rate(&self) -> u16 {
        self.refresh_rate
    }

    #[inline]
    pub fn monitor(&self) -> RootMonitorHandle {
        RootMonitorHandle {
            inner: PlatformMonitorHandle::Wayland(self.monitor.clone()),
        }
    }
}

#[derive(Clone)]
pub struct MonitorHandle {
    pub(crate) proxy: wl_output::WlOutput,
}

impl PartialEq for MonitorHandle {
    fn eq(&self, other: &Self) -> bool {
        self.native_identifier() == other.native_identifier()
    }
}

impl Eq for MonitorHandle {}

impl PartialOrd for MonitorHandle {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(&other))
    }
}

impl Ord for MonitorHandle {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.native_identifier().cmp(&other.native_identifier())
    }
}

impl std::hash::Hash for MonitorHandle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.native_identifier().hash(state);
    }
}

impl fmt::Debug for MonitorHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[derive(Debug)]
        struct MonitorHandle {
            name: Option<String>,
            native_identifier: u32,
            size: PhysicalSize<u32>,
            position: PhysicalPosition<i32>,
            scale_factor: i32,
        }

        let monitor_id_proxy = MonitorHandle {
            name: self.name(),
            native_identifier: self.native_identifier(),
            size: self.size(),
            position: self.position(),
            scale_factor: self.scale_factor(),
        };

        monitor_id_proxy.fmt(f)
    }
}

impl MonitorHandle {
    pub fn name(&self) -> Option<String> {
        with_output_info(&self.proxy, |info| {
            format!("{} ({})", info.model, info.make)
        })
    }

    #[inline]
    pub fn native_identifier(&self) -> u32 {
        with_output_info(&self.proxy, |info| info.id).unwrap_or(0)
    }

    pub fn size(&self) -> PhysicalSize<u32> {
        match with_output_info(&self.proxy, |info| {
            info.modes
                .iter()
                .find(|m| m.is_current)
                .map(|m| m.dimensions)
        }) {
            Some(Some((w, h))) => (w as u32, h as u32),
            _ => (0, 0),
        }
        .into()
    }

    pub fn position(&self) -> PhysicalPosition<i32> {
        with_output_info(&self.proxy, |info| info.location)
            .unwrap_or((0, 0))
            .into()
    }

    #[inline]
    pub fn scale_factor(&self) -> i32 {
        with_output_info(&self.proxy, |info| info.scale_factor).unwrap_or(1)
    }

    #[inline]
    pub fn video_modes(&self) -> impl Iterator<Item = RootVideoMode> {
        let monitor = self.clone();

        with_output_info(&self.proxy, |info| info.modes.clone())
            .unwrap_or(vec![])
            .into_iter()
            .map(move |x| RootVideoMode {
                video_mode: PlatformVideoMode::Wayland(VideoMode {
                    size: (x.dimensions.0 as u32, x.dimensions.1 as u32),
                    refresh_rate: (x.refresh_rate as f32 / 1000.0).round() as u16,
                    bit_depth: 32,
                    monitor: monitor.clone(),
                }),
            })
    }
}

pub fn primary_monitor(outputs: &Vec<wl_output::WlOutput>) -> MonitorHandle {
    outputs
        .iter()
        .next()
        .map(|output| MonitorHandle {
            proxy: output.clone(),
        })
        .expect("No monitor is available.")
}

pub fn available_monitors(outputs: &Vec<wl_output::WlOutput>) -> VecDeque<MonitorHandle> {
    outputs
        .iter()
        .map(|output| MonitorHandle {
            proxy: output.clone(),
        })
        .collect()
}
