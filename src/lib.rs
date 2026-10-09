//! Glint UI: a refined component library for GPUI-CE.
//!
//! Import controls and their events from [`components`], and use
//! [`theme::ThemePreset`] for consistent light and dark styles. The host application
//! owns window creation, state, persistence, and GPUI's asset source.
//!
//! Register [`components::GlintAssets`] to load embedded icons and bind the keyboard
//! shortcuts required by the editors your application uses. See the
//! [component guide](https://github.com/yanguiyuan/gpui-glint/blob/main/docs/components.md)
//! and [examples](https://github.com/yanguiyuan/gpui-glint/tree/main/examples)
//! for integration patterns.
//!
//! Enable the optional `webview` feature for native webview integration on macOS
//! and Windows.
pub mod components;
pub mod theme;
