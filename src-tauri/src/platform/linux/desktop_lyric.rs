// Copyright 2026 Atemukesu
// SPDX-License-Identifier: GPL-3.0-only

//! Transparent, frameless desktop-lyric layer on Linux.
//!
//! CEF cannot paint a transparent *windowed* browser on Linux: a windowed
//! browser owns a native X11 child window and Chromium fills it with an opaque
//! surface. Transparency only exists in off-screen rendering (OSR), where the
//! application owns the window and CEF hands over the painted pixels with a
//! real alpha channel. This module is that renderer:
//!
//! * the floating window is an ordinary GTK4 toplevel created by Tauri, made
//!   transparent by GTK and the X11 ARGB visual;
//! * its single webview is created with [`Offscreen`], so CEF creates no native
//!   view and paints into a BGRA buffer instead;
//! * every frame arrives in `RenderHandler::on_paint`, becomes a GDK
//!   `MemoryTexture` (CEF's buffer is already premultiplied BGRA), and is shown
//!   by a `gtk::Picture` that GTK composites with alpha;
//! * pointer, keyboard, focus and redraw events arrive through
//!   [`Cef::on_window_event`] and are forwarded to the browser host ourselves.
//!
//! The runtime pins the app to X11 (`GDK_BACKEND=x11`, `--ozone-platform=x11`),
//! so this always lands on X11 or XWayland and never has to deal with Wayland
//! subsurfaces. Off-screen webviews also cannot be reparented, which is why the
//! overlay window is dedicated: its webview is the only one it will ever host.
//!
//! CEF reports the view rect in DIPs but paints physical pixels, so the two are
//! mapped through the window's scale factor.

use std::{
  cell::RefCell,
  collections::HashMap,
  sync::{Arc, Mutex},
};

use gtk::prelude::*;
use tauri::{AppHandle, Manager, WebviewWindow, WebviewWindowBuilder};
use tauri_runtime_cef::{
  AsCefWebviewAttributes, Offscreen, OffscreenView,
  cef::{
    Browser, BrowserHost, CursorType, ImplBrowser, ImplBrowserHost, ImplRenderHandler, KeyEvent,
    KeyEventType, MouseButtonType, MouseEvent, PaintElementType, Rect as CefRect, RenderHandler,
    ScreenInfo, WrapRenderHandler, rc::Rc as _,
    sys::cef_event_flags_t,
    wrap_render_handler,
  },
};
use winit::{
  cursor::CursorIcon,
  event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
  event_loop::ActiveEventLoop,
  keyboard::{Key, ModifiersState, NamedKey},
  window::Window,
};

/// Label of the floating overlay window.
pub const WINDOW_LABEL: &str = crate::commands::desktop_lyric::WINDOW_LABEL;

/// One painted frame, exactly as CEF handed it over: premultiplied BGRA.
struct Frame {
  width: usize,
  height: usize,
  pixels: Vec<u8>,
}

impl Frame {
  /// Wraps the buffer as a GDK texture. `MemoryTexture` borrows the bytes for
  /// its whole life, and `from_owned` copies them into a GBytes it frees with
  /// the texture.
  fn texture(&self) -> Option<gtk::gdk::MemoryTexture> {
    if self.width == 0 || self.height == 0 {
      return None;
    }
    let bytes = gtk::glib::Bytes::from_owned(self.pixels.clone());
    Some(gtk::gdk::MemoryTexture::new(
      self.width as i32,
      self.height as i32,
      gtk::gdk::MemoryFormat::B8g8r8a8Premultiplied,
      &bytes,
      self.width * 4,
    ))
  }
}

/// The main-thread state of one off-screen overlay.
struct Embedded {
  view: OffscreenView,
  frames: Arc<Mutex<Option<Frame>>>,
  browser: Option<Browser>,
  /// The `gtk::Picture` that shows the frames, once the window is built.
  picture: Option<gtk::Picture>,
  /// The last pointer position, in view DIPs, for `PointerLeft` and the wheel.
  pointer: (i32, i32),
  modifiers: ModifiersState,
  buttons: u32,
}

thread_local! {
  /// Overlays by window label. Every CEF and winit callback in this module runs
  /// on the main thread, so a thread local is the cheap place for this state.
  /// It is never borrowed across a call into CEF, which may call the render
  /// handler back synchronously.
  static EMBEDDED: RefCell<HashMap<String, Embedded>> = RefCell::default();
}

wrap_render_handler! {
  // Receives every frame CEF paints for one overlay.
  struct OverlayRenderHandler {
    label: String,
    view: OffscreenView,
    frames: Arc<Mutex<Option<Frame>>>,
  }

  impl RenderHandler {
    fn view_rect(&self, browser: Option<&mut Browser>, rect: Option<&mut CefRect>) {
      self.remember(browser);
      let Some(rect) = rect else { return };
      let Some(window) = self.view.window() else { return };
      let (width, height) = viewport_dips(window.as_ref());
      // CEF asserts on an empty view, which is what a minimized window has.
      *rect = CefRect {
        x: 0,
        y: 0,
        width: (width.round() as i32).max(1),
        height: (height.round() as i32).max(1),
      };
    }

    fn screen_info(
      &self,
      _browser: Option<&mut Browser>,
      screen_info: Option<&mut ScreenInfo>,
    ) -> std::os::raw::c_int {
      let (Some(info), Some(window)) = (screen_info, self.view.window()) else {
        return 0;
      };
      // What CEF scales the paint buffer by: the view rect is in DIPs, the
      // pixels it paints are physical.
      info.device_scale_factor = window.scale_factor() as f32;
      1
    }

    fn on_paint(
      &self,
      browser: Option<&mut Browser>,
      type_: PaintElementType,
      _dirty_rects: Option<&[CefRect]>,
      buffer: *const u8,
      width: std::os::raw::c_int,
      height: std::os::raw::c_int,
    ) {
      self.remember(browser);
      // A popup widget (an open `<select>`) is a second paint target the
      // overlay has no use for.
      if type_ != PaintElementType::default() {
        return;
      }
      let (Ok(width), Ok(height)) = (usize::try_from(width), usize::try_from(height)) else {
        return;
      };
      if buffer.is_null() || width == 0 || height == 0 {
        return;
      }
      // CEF only lends the buffer for the duration of this call.
      let pixels = unsafe { std::slice::from_raw_parts(buffer, width * height * 4) };
      if std::env::var_os("WELKIN_DEBUG_LYRIC").is_some() {
        let opaque = pixels.chunks_exact(4).filter(|px| px[3] == 255).count();
        let any = pixels.chunks_exact(4).filter(|px| px[3] != 0).count();
        eprintln!(
          "osr frame {width}x{height} alpha>0: {any}/{} ({}%) opaque: {opaque}",
          width * height,
          any * 100 / pixels.len().max(1) * 4,
        );
      }
      *self.frames.lock().unwrap_or_else(|e| e.into_inner()) = Some(Frame {
        width,
        height,
        pixels: pixels.to_vec(),
      });
      self.view.request_redraw();
    }
  }
}

impl OverlayRenderHandler {
  /// Keeps the browser the render handler is first called with: it is how the
  /// input forwarding below reaches the browser host.
  fn remember(&self, browser: Option<&mut Browser>) {
    let Some(browser) = browser else { return };
    EMBEDDED.with_borrow_mut(|embedded| {
      if let Some(embedded) = embedded.get_mut(&self.label) {
        if embedded.browser.is_none() {
          embedded.browser = Some(browser.clone());
        }
      }
    });
  }
}

/// The viewport in DIPs: the window's real surface size, not the webview bounds.
///
/// `OffscreenView::bounds` is written by the runtime's layout pass and can lag a
/// resize by a frame; reading it from `view_rect` would then lay the page out for
/// a stale height while the window already has the new one, and the frame would
/// arrive with the wrong aspect ratio. The surface size always matches the window
/// being painted into, so it is the authoritative source.
fn viewport_dips(window: &dyn Window) -> (f64, f64) {
  let physical: (u32, u32) = window.surface_size().into();
  let scale = window.scale_factor();
  (
    physical.0 as f64 / scale,
    physical.1 as f64 / scale,
  )
}

/// Builds the overlay window, or returns the existing one.
///
/// `on_page_load` runs when the overlay's webview finishes loading, so the
/// caller can replay documents pushed before the window existed.
///
/// Must run on the main thread: `WebviewWindowBuilder::build` waits for it, so
/// the caller is expected to be an `async` command.
pub fn open<F>(app: &AppHandle, on_page_load: F) -> Result<WebviewWindow, String>
where
  F: Fn(WebviewWindow, tauri::webview::PageLoadPayload<'_>) + Send + Sync + 'static,
{
  if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
    return Ok(window);
  }

  let frames: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
  let mut offscreen = Offscreen::new({
    let frames = frames.clone();
    // Runs on CEF's UI thread, which the runtime drives from the main thread;
    // that is why the GTK widgets below are created here rather than in `open`,
    // which a tokio worker calls.
    move |view| {
      EMBEDDED.with_borrow_mut(|embedded| {
        embedded.insert(
          WINDOW_LABEL.to_string(),
          Embedded {
            view: view.clone(),
            frames: frames.clone(),
            browser: None,
            picture: None,
            pointer: (0, 0),
            modifiers: ModifiersState::empty(),
            buttons: 0,
          },
        );
      });
      OverlayRenderHandler::new(WINDOW_LABEL.to_string(), view, frames.clone())
    }
  });
  // Without a native view Chromium has nowhere to set the cursor itself, so it
  // asks, and the request is applied to the GTK window.
  offscreen.cursor_handler = Some(Arc::new(|cursor: CursorType, _| {
    let icon = match cursor {
      CursorType::HAND => CursorIcon::Pointer,
      CursorType::IBEAM => CursorIcon::Text,
      _ => CursorIcon::Default,
    };
    EMBEDDED.with_borrow(|embedded| {
      if let Some(window) = embedded
        .get(WINDOW_LABEL)
        .and_then(|embedded| embedded.view.window())
      {
        window.set_cursor(icon.into());
      }
    });
  }));

  let mut builder = WebviewWindowBuilder::new(
    app,
    WINDOW_LABEL,
    tauri::WebviewUrl::App("desktop-lyric-overlay.html".into()),
  )
  .title("Welkin Desktop Lyrics")
  .inner_size(920.0, 240.0)
  .min_inner_size(240.0, 72.0)
  .decorations(false)
  .transparent(true)
  .shadow(false)
  .resizable(true)
  .always_on_top(true)
  .skip_taskbar(true)
  .focused(false)
  .on_page_load(on_page_load)
  // One off-screen webview always fills the window it is the only child of.
  .auto_resize();

  match builder
    .runtime_specific_attributes_mut()
    .as_cef_webview_attributes_mut()
  {
    Some(attributes) => attributes.offscreen = Some(offscreen),
    None => return Err("not running on the CEF runtime".into()),
  }

  let window = builder
    .build()
    .map_err(|error| format!("create desktop lyric window failed: {error}"))?;

  // GTK may only be touched from the main thread, and this function runs on a
  // tokio worker when the command is `async`, so both the presenter and the
  // initial placement happen there.
  if let Err(error) = app.run_on_main_thread({
    let app = app.clone();
    move || {
      let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
        return;
      };
      attach_presenter(&window, WINDOW_LABEL);
      place_default(&window);
    }
  }) {
    log::error!("failed to schedule the desktop lyric presenter: {error}");
  }

  Ok(window)
}

/// Puts the window near the bottom-centre of the primary monitor.
pub fn place_default(window: &WebviewWindow) {
  let Ok(Some(monitor)) = window.current_monitor() else {
    return;
  };
  let size = monitor.size();
  let scale = monitor.scale_factor();
  let width = 920.0_f64;
  let x = (size.width as f64 / scale - width) / 2.0;
  let y = size.height as f64 / scale - 240.0 - 72.0;
  let _ = window.set_position(tauri::LogicalPosition::new(x.max(0.0), y.max(0.0)));
}

/// Hangs the `gtk::Picture` that shows the frame over the window's content
/// area.
///
/// The GTK window is transparent (winit adds the transparency CSS class and
/// Tauri asks for an ARGB visual), so whatever the page paints as transparent
/// stays transparent and shows the desktop underneath.
///
/// The picture goes into a `gtk::Overlay` rather than into the window's default
/// box: that box already holds the CEF X11 host, and appending next to it would
/// split the window between the two, so the frame would be scaled into half of
/// it. An overlay gives the picture the full content area and stacks it above
/// the (unmapped, for an off-screen-only window) host.
fn attach_presenter(window: &WebviewWindow, label: &str) {
  let Ok(gtk_window) = window.gtk_window() else {
    return;
  };
  let Ok(vbox) = window.default_vbox() else {
    return;
  };

  let picture = gtk::Picture::new();
  // `Contain` keeps the frame's aspect ratio: with `Fill` any mismatch between
  // the painted frame and the widget (a resize not yet repainted) would scale
  // the image non-uniformly, which squashes the text instead of just cropping.
  picture.set_content_fit(gtk::ContentFit::Contain);
  picture.set_can_shrink(true);
  picture.set_hexpand(true);
  picture.set_vexpand(true);
  // The picture only presents pixels: winit installed the controllers that
  // produce the window events this module forwards, so it must not take input.
  picture.set_can_target(false);

  let overlay = gtk::Overlay::new();
  overlay.set_hexpand(true);
  overlay.set_vexpand(true);
  overlay.set_child(Some(&vbox));
  overlay.add_overlay(&picture);
  // Replaces the window's child with the overlay; the box is reparented under
  // it, so the runtime's own widgets keep their place.
  gtk_window.set_child(Some(&overlay));
  overlay.set_measure_overlay(&picture, true);
  overlay.set_clip_overlay(&picture, false);

  EMBEDDED.with_borrow_mut(|embedded| {
    if let Some(embedded) = embedded.get_mut(label) {
      embedded.picture = Some(picture);
    }
  });
}

/// Puts the newest frame into the window, if there is one.
fn present(label: &str) {
  let Some((picture, frames)) = EMBEDDED.with_borrow(|embedded| {
    let embedded = embedded.get(label)?;
    Some((embedded.picture.clone()?, embedded.frames.clone()))
  }) else {
    return;
  };
  let Some(texture) = frames
    .lock()
    .unwrap_or_else(|e| e.into_inner())
    .as_ref()
    .and_then(Frame::texture)
  else {
    return;
  };
  picture.set_paintable(Some(&texture));
}

/// Whether a native window event belongs to an off-screen overlay in this
/// module, and pushes the matching state into CEF.
///
/// Registered through [`tauri_runtime_cef::Cef::on_window_event`], so it runs
/// before the runtime handles the event; the runtime's own handling (closing,
/// resizing) still happens afterwards.
pub fn on_window_event(
  label: &str,
  _event_loop: &dyn ActiveEventLoop,
  window: Arc<dyn Window>,
  event: &WindowEvent,
) {
  if !EMBEDDED.with_borrow(|embedded| embedded.contains_key(label)) {
    return;
  }

  if let WindowEvent::Destroyed = event {
    EMBEDDED.with_borrow_mut(|embedded| embedded.remove(label));
    return;
  }

  let Some(host) = EMBEDDED.with_borrow(|embedded| {
    embedded
      .get(label)
      .and_then(|embedded| embedded.browser.as_ref())
      .and_then(|browser| browser.host())
  }) else {
    return;
  };

  match event {
    WindowEvent::RedrawRequested => present(label),
    // The window changed size: the viewport CEF laid out is stale, so ask it to
    // relayout and repaint before the new frame is presented.
    WindowEvent::SurfaceResized(_) => {
      host.was_resized();
      host.invalidate(PaintElementType::default());
    }
    WindowEvent::Focused(focused) => {
      host.set_focus(i32::from(*focused));
    }
    WindowEvent::ModifiersChanged(modifiers) => {
      with_embedded(label, |embedded| embedded.modifiers = modifiers.state());
    }
    WindowEvent::PointerMoved { position, .. } => {
      let event = mouse_event(label, &window, position.x, position.y);
      host.send_mouse_move_event(Some(&event), 0);
    }
    WindowEvent::PointerLeft { .. } => {
      let event = with_embedded(label, |embedded| MouseEvent {
        x: embedded.pointer.0,
        y: embedded.pointer.1,
        modifiers: flags(embedded),
      });
      if let Some(event) = event {
        host.send_mouse_move_event(Some(&event), 1);
      }
    }
    WindowEvent::PointerButton {
      state, position, button, ..
    } => {
      let (kind, flag) = match button.clone().mouse_button() {
        Some(MouseButton::Left) => (MouseButtonType::LEFT, 16),
        Some(MouseButton::Middle) => (MouseButtonType::MIDDLE, 32),
        Some(MouseButton::Right) => (MouseButtonType::RIGHT, 64),
        _ => return,
      };
      let pressed = *state == ElementState::Pressed;
      with_embedded(label, |embedded| {
        if pressed {
          embedded.buttons |= flag;
        } else {
          embedded.buttons &= !flag;
        }
      });
      let event = mouse_event(label, &window, position.x, position.y);
      host.send_mouse_click_event(Some(&event), kind, i32::from(!pressed), 1);
    }
    WindowEvent::MouseWheel { delta, .. } => {
      // Chromium scrolls by pixels; a wheel notch is 120 of them.
      let (dx, dy) = match delta {
        MouseScrollDelta::LineDelta(x, y) => (x * 120., y * 120.),
        MouseScrollDelta::PixelDelta(delta) => (delta.x as f32, delta.y as f32),
        _ => return,
      };
      // GDK's deltas are positive downwards, the opposite of CEF's convention.
      let (dx, dy) = (-dx, -dy);
      let event = with_embedded(label, |embedded| MouseEvent {
        x: embedded.pointer.0,
        y: embedded.pointer.1,
        modifiers: flags(embedded),
      });
      if let Some(event) = event {
        host.send_mouse_wheel_event(Some(&event), dx as i32, dy as i32);
      }
    }
    WindowEvent::KeyboardInput { event, .. } => {
      let modifiers = with_embedded(label, |embedded| flags(embedded)).unwrap_or(0);
      let pressed = event.state == ElementState::Pressed;
      host.send_key_event(Some(&KeyEvent {
        type_: if pressed {
          KeyEventType::RAWKEYDOWN
        } else {
          KeyEventType::KEYUP
        },
        modifiers,
        windows_key_code: windows_key_code(&event.logical_key),
        ..Default::default()
      }));
      // Text goes in as character events; a shortcut is not text.
      let shortcut = modifiers & (CONTROL_DOWN | COMMAND_DOWN) != 0;
      if pressed && !shortcut {
        if let Some(text) = &event.text {
          send_text(&host, text, modifiers);
        }
      }
    }
    // What an input method composed.
    WindowEvent::Ime(Ime::Commit(text)) => {
      send_text(&host, text, 0);
    }
    _ => {}
  }
}

// `c_uint` everywhere but Windows, where bindgen makes it a `c_int`.
const SHIFT_DOWN: u32 = cef_event_flags_t::EVENTFLAG_SHIFT_DOWN.0 as u32;
const CONTROL_DOWN: u32 = cef_event_flags_t::EVENTFLAG_CONTROL_DOWN.0 as u32;
const ALT_DOWN: u32 = cef_event_flags_t::EVENTFLAG_ALT_DOWN.0 as u32;
const COMMAND_DOWN: u32 = cef_event_flags_t::EVENTFLAG_COMMAND_DOWN.0 as u32;

/// CEF's event flags for the held modifiers and mouse buttons.
fn flags(embedded: &Embedded) -> u32 {
  let mut flags = embedded.buttons;
  for (held, flag) in [
    (embedded.modifiers.shift_key(), SHIFT_DOWN),
    (embedded.modifiers.control_key(), CONTROL_DOWN),
    (embedded.modifiers.alt_key(), ALT_DOWN),
    (embedded.modifiers.meta_key(), COMMAND_DOWN),
  ] {
    if held {
      flags |= flag;
    }
  }
  flags
}

fn with_embedded<T>(label: &str, f: impl FnOnce(&mut Embedded) -> T) -> Option<T> {
  EMBEDDED.with_borrow_mut(|embedded| embedded.get_mut(label).map(f))
}

/// A mouse event at a physical window position, in the view's DIPs.
///
/// The view fills the window, so the window origin is the view origin; only the
/// scale factor has to be undone.
fn mouse_event(label: &str, window: &Arc<dyn Window>, x: f64, y: f64) -> MouseEvent {
  let scale = window.scale_factor();
  let position = ((x / scale).round() as i32, (y / scale).round() as i32);
  with_embedded(label, |embedded| {
    embedded.pointer = position;
    MouseEvent {
      x: position.0,
      y: position.1,
      modifiers: flags(embedded),
    }
  })
  .unwrap_or_default()
}

fn send_text(host: &BrowserHost, text: &str, modifiers: u32) {
  for unit in text.encode_utf16() {
    host.send_key_event(Some(&KeyEvent {
      type_: KeyEventType::CHAR,
      modifiers,
      windows_key_code: i32::from(unit),
      character: unit,
      unmodified_character: unit,
      ..Default::default()
    }));
  }
}

/// The Windows virtual-key code Chromium identifies a key by, for the keys an
/// editable field needs. Letters and digits are their uppercase ASCII code.
fn windows_key_code(key: &Key) -> i32 {
  match key {
    Key::Named(named) => match named {
      NamedKey::Backspace => 0x08,
      NamedKey::Tab => 0x09,
      NamedKey::Enter => 0x0D,
      NamedKey::Shift => 0x10,
      NamedKey::Control => 0x11,
      NamedKey::Alt => 0x12,
      NamedKey::Escape => 0x1B,
      NamedKey::PageUp => 0x21,
      NamedKey::PageDown => 0x22,
      NamedKey::End => 0x23,
      NamedKey::Home => 0x24,
      NamedKey::ArrowLeft => 0x25,
      NamedKey::ArrowUp => 0x26,
      NamedKey::ArrowRight => 0x27,
      NamedKey::ArrowDown => 0x28,
      NamedKey::Delete => 0x2E,
      _ => 0,
    },
    Key::Character(text) => text
      .chars()
      .next()
      .filter(|c| c.is_ascii_alphanumeric() || *c == ' ')
      .map_or(0, |c| c.to_ascii_uppercase() as i32),
    _ => 0,
  }
}
