//! Optional native child WebView. Keep one Entity per parent Window and hide it
//! when its layout node is removed. Native views sit above GPUI-painted overlays.
use gpui::prelude::FluentBuilder;
use gpui::{
    Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Window, canvas, div, px, rgb,
};
use std::{cell::RefCell, rc::Rc};
pub use wry;
use wry::raw_window_handle::HasWindowHandle;

pub struct WebView {
    native: Rc<wry::WebView>,
    focus: FocusHandle,
    visible: bool,
    bounds: Rc<RefCell<Option<gpui::Bounds<gpui::Pixels>>>>,
    error: Rc<RefCell<Option<String>>>,
}
impl WebView {
    /// Configure navigation, IPC and permissions on the builder before creating.
    /// Must be called on the UI thread with the owning live Window.
    pub fn new(
        builder: wry::WebViewBuilder<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Self, String> {
        let handle = window.window_handle().map_err(|e| e.to_string())?;
        let native = builder
            .with_visible(false)
            .build_as_child(&handle)
            .map_err(|e| e.to_string())?;
        Ok(Self::from_native(native, cx))
    }
    /// Wrap a Wry child view already created for this window on the UI thread.
    pub fn from_native(native: wry::WebView, cx: &mut Context<Self>) -> Self {
        Self {
            native: Rc::new(native),
            focus: cx.focus_handle(),
            visible: true,
            bounds: Rc::new(RefCell::new(None)),
            error: Rc::new(RefCell::new(None)),
        }
    }
    /// Passive HTML document: scripts, navigations, new windows and downloads disabled.
    /// Use `new` with a host-configured builder for interactive browser behavior.
    pub fn html(html: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<Self, String> {
        Self::new(
            wry::WebViewBuilder::new()
                .with_html(html)
                .with_incognito(true)
                .with_javascript_disabled()
                .with_navigation_handler(|url| url == "about:blank")
                .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
                .with_download_started_handler(|_, _| false),
            window,
            cx,
        )
    }
    pub fn load_html(&mut self, html: &str, cx: &mut Context<Self>) -> Result<(), String> {
        self.native.load_html(html).map_err(|e| e.to_string())?;
        *self.error.borrow_mut() = None;
        cx.notify();
        Ok(())
    }
    pub fn load_url(&mut self, url: &str, cx: &mut Context<Self>) -> Result<(), String> {
        self.native.load_url(url).map_err(|e| e.to_string())?;
        *self.error.borrow_mut() = None;
        cx.notify();
        Ok(())
    }
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) -> Result<(), String> {
        if self.visible == visible {
            return Ok(());
        }
        if !visible {
            self.native.set_visible(false).map_err(|e| e.to_string())?;
            self.native.focus_parent().map_err(|e| e.to_string())?;
        }
        self.visible = visible;
        cx.notify();
        Ok(())
    }
    pub fn visible(&self) -> bool {
        self.visible
    }
    pub fn raw(&self) -> &wry::WebView {
        &self.native
    }
}
impl Focusable for WebView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Drop for WebView {
    fn drop(&mut self) {
        let _ = self.native.set_visible(false);
    }
}
impl Render for WebView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let native = self.native.clone();
        let bounds = self.bounds.clone();
        let error = self.error.clone();
        let visible = self.visible;
        let owner: Entity<Self> = cx.entity();
        let layout = canvas(
            move |area, window, cx| {
                if !visible {
                    return;
                }
                let clipped = area.intersect(&window.content_mask().bounds);
                let result = if clipped.size.width <= px(0.) || clipped.size.height <= px(0.) {
                    native.set_visible(false)
                } else {
                    let changed = bounds.borrow().as_ref() != Some(&clipped);
                    let result = if changed {
                        native.set_bounds(wry::Rect {
                            position: wry::dpi::LogicalPosition::new(
                                f64::from(f32::from(clipped.origin.x)),
                                f64::from(f32::from(clipped.origin.y)),
                            )
                            .into(),
                            size: wry::dpi::LogicalSize::new(
                                f64::from(f32::from(clipped.size.width)),
                                f64::from(f32::from(clipped.size.height)),
                            )
                            .into(),
                        })
                    } else {
                        Ok(())
                    };
                    result.and_then(|_| {
                        *bounds.borrow_mut() = Some(clipped);
                        native.set_visible(true)
                    })
                };
                if let Err(problem) = result {
                    let _ = native.set_visible(false);
                    let message = problem.to_string();
                    if error.borrow().as_ref() != Some(&message) {
                        *error.borrow_mut() = Some(message);
                        cx.notify(owner.entity_id());
                    }
                }
            },
            |_, _, _, _| {},
        )
        .size_full();
        let problem = self.error.borrow().clone();
        div()
            .size_full()
            .track_focus(&self.focus)
            .when_some(problem.clone(), |el, problem| {
                el.child(
                    div()
                        .p_3()
                        .text_color(rgb(0xd9342b))
                        .child(format!("WebView: {problem}")),
                )
            })
            .when(problem.is_none(), |el| el.child(layout))
    }
}
