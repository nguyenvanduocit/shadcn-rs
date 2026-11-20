//! The native window: winit for events, softbuffer to show a CPU-painted
//! pixmap. Positions handed on are logical pixels; the pixmap is physical.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use softbuffer::{Context, Surface};
use tiny_skia::Pixmap;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId};

use crate::convert::f64_to_f32;
use crate::events::{KeyEvent, Modifiers};
use crate::interact::LINE_SCROLL;
use crate::style::Cursor;

/// How often an animating window repaints.
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

/// Input the window passes to its [`WindowHandler`], in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    PointerMoved {
        x: f32,
        y: f32,
    },
    PointerLeft,
    MouseDown {
        x: f32,
        y: f32,
    },
    MouseUp {
        x: f32,
        y: f32,
    },
    /// The secondary (right) button went down.
    SecondaryDown {
        x: f32,
        y: f32,
    },
    /// Scroll by this many pixels (positive moves the content up / left).
    Wheel {
        dx: f32,
        dy: f32,
    },
    Key(KeyEvent),
    Modifiers(Modifiers),
}

/// When the window should wake next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Wake {
    pub redraw: bool,
    /// Paint again soon even without input (an animation is running).
    pub animating: bool,
    pub next_timer: Option<Instant>,
}

/// What runs inside the window.
pub trait WindowHandler {
    /// Paint one frame; `scale` is physical pixels per logical pixel.
    fn draw(&mut self, pixmap: &mut Pixmap, scale: f32);
    /// React to `input`; return `true` when the window must be redrawn.
    fn input(&mut self, input: Input) -> bool;
    /// The cursor to show where the pointer is.
    fn cursor(&self) -> Cursor;
    /// Called when the event queue is empty: run due work and say when to wake.
    fn idle(&mut self) -> Wake;
}

struct WindowState {
    window: Arc<Window>,
    #[expect(
        dead_code,
        reason = "held as a keep-alive: the surface is valid only while its context lives"
    )]
    context: Context<Arc<Window>>,
    surface: Surface<Arc<Window>, Arc<Window>>,
    pixmap: Option<Pixmap>,
}

struct Handler<H: WindowHandler> {
    app: H,
    title: String,
    state: Option<WindowState>,
    /// Last pointer position, in logical pixels.
    cursor: (f32, f32),
    modifiers: Modifiers,
}

/// Open a window titled `title` and run `handler` until it closes.
pub fn run(title: &str, handler: impl WindowHandler + 'static) -> ! {
    let event_loop = match EventLoop::new() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("failed to create event loop: {e}");
            std::process::exit(1);
        }
    };
    let mut handler = Handler {
        app: handler,
        title: title.to_owned(),
        state: None,
        cursor: (0.0, 0.0),
        modifiers: Modifiers::default(),
    };
    if let Err(e) = event_loop.run_app(&mut handler) {
        eprintln!("event loop error: {e}");
        std::process::exit(1);
    }
    std::process::exit(0)
}

fn cursor_icon(c: Cursor) -> CursorIcon {
    match c {
        Cursor::Default => CursorIcon::Default,
        Cursor::Pointer => CursorIcon::Pointer,
        Cursor::Text => CursorIcon::Text,
        Cursor::NotAllowed => CursorIcon::NotAllowed,
        Cursor::Grab => CursorIcon::Grab,
        Cursor::ResizeColumn => CursorIcon::ColResize,
        Cursor::ResizeRow => CursorIcon::RowResize,
    }
}

impl<H: WindowHandler> Handler<H> {
    fn send(&mut self, input: Input) {
        let redraw = self.app.input(input);
        if let Some(state) = self.state.as_ref() {
            state.window.set_cursor(cursor_icon(self.app.cursor()));
            if redraw {
                state.window.request_redraw();
            }
        }
    }

    fn scale(&self) -> f32 {
        self.state.as_ref().map_or(1.0, |s| f64_to_f32(s.window.scale_factor()))
    }
}

impl<H: WindowHandler> ApplicationHandler for Handler<H> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = WindowAttributes::default()
            .with_title(self.title.clone())
            .with_inner_size(LogicalSize::new(1280.0, 820.0));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("create_window: {e}");
                event_loop.exit();
                return;
            }
        };
        let context = match Context::new(Arc::clone(&window)) {
            Ok(ctx) => ctx,
            Err(e) => {
                eprintln!("softbuffer context: {e}");
                event_loop.exit();
                return;
            }
        };
        let surface = match Surface::new(&context, Arc::clone(&window)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("softbuffer surface: {e}");
                event_loop.exit();
                return;
            }
        };
        window.request_redraw();
        self.state = Some(WindowState {
            window,
            context,
            surface,
            pixmap: None,
        });
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let scale = self.scale();
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(state) = self.state.as_ref() {
                    state.window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let p = (f64_to_f32(position.x) / scale, f64_to_f32(position.y) / scale);
                self.cursor = p;
                self.send(Input::PointerMoved { x: p.0, y: p.1 });
            }
            WindowEvent::CursorLeft { .. } => self.send(Input::PointerLeft),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.cursor;
                self.send(match state {
                    ElementState::Pressed => Input::MouseDown { x, y },
                    ElementState::Released => Input::MouseUp { x, y },
                });
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                let (x, y) = self.cursor;
                self.send(Input::SecondaryDown { x, y });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * LINE_SCROLL, -y * LINE_SCROLL),
                    MouseScrollDelta::PixelDelta(p) => (-f64_to_f32(p.x) / scale, -f64_to_f32(p.y) / scale),
                };
                self.send(Input::Wheel { dx, dy });
            }
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.modifiers = Modifiers {
                    shift: s.contains(ModifiersState::SHIFT),
                    control: s.contains(ModifiersState::CONTROL),
                    alt: s.contains(ModifiersState::ALT),
                    command: s.contains(ModifiersState::SUPER),
                };
                self.send(Input::Modifiers(self.modifiers));
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.send(Input::Key(KeyEvent {
                    key: event.logical_key,
                    text: event.text.map(|t| t.to_string()),
                    modifiers: self.modifiers,
                }));
            }
            WindowEvent::RedrawRequested => self.redraw(scale),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let wake = self.app.idle();
        if let Some(state) = self.state.as_ref()
            && (wake.redraw || wake.animating)
        {
            state.window.request_redraw();
        }
        let next = if wake.animating {
            Instant::now().checked_add(FRAME_INTERVAL)
        } else {
            None
        };
        match next.into_iter().chain(wake.next_timer).min() {
            Some(at) => event_loop.set_control_flow(ControlFlow::WaitUntil(at)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

impl<H: WindowHandler> Handler<H> {
    fn redraw(&mut self, scale: f32) {
        let app = &mut self.app;
        let Some(state) = self.state.as_mut() else { return };
        let size = state.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return;
        };
        if let Err(e) = state.surface.resize(w, h) {
            eprintln!("surface resize: {e}");
            return;
        }
        let fits = state
            .pixmap
            .as_ref()
            .is_some_and(|p| p.width() == size.width && p.height() == size.height);
        if !fits {
            state.pixmap = Pixmap::new(size.width, size.height);
        }
        let Some(pixmap) = state.pixmap.as_mut() else { return };
        app.draw(pixmap, scale);
        match state.surface.buffer_mut() {
            Ok(mut buf) => {
                for (dst, src) in buf.iter_mut().zip(pixmap.pixels()) {
                    *dst = u32::from_be_bytes([0, src.red(), src.green(), src.blue()]);
                }
                if let Err(e) = buf.present() {
                    eprintln!("present: {e}");
                }
            }
            Err(e) => eprintln!("buffer_mut: {e}"),
        }
    }
}
