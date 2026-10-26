# Glint Agent icon

Companion to the Glint API icon: the same #2563EB tile, 880 × 880 bounds at
(72, 72), 200px corner radius, white glyph, 68px rounded strokes, and transparent
exterior on a 1024 × 1024 canvas. No gradients, glass effects, shadows, or text.

The robot's antenna, two eyes, and short mouth distinguish the Agent from the
API application's opposing brackets. The compact symmetrical face stays legible
in the topbar and Dock.

`glint-agent-icon.svg` is the editable source. The PNG is rendered from that SVG
with resvg; the ICO contains 16, 24, 32, 48, 64, 128, and 256px sizes. The ICNS
contains standard and Retina macOS sizes through 1024px.

The PNG is embedded for the application topbar and the macOS Dock, including
direct `cargo run -p glint-agent` launches. The ICO is embedded in Windows
executables by `build.rs`. The ICNS is available for macOS application bundles.
