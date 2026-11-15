mod badges;
mod buttons;
mod common;
mod icons;
mod inputs;
mod layout;
mod overview;
mod tabs;
mod themes;

pub use badges::BadgesPage;
pub use buttons::ButtonsPage;
pub use icons::IconsPage;
pub use inputs::InputsPage;
pub use layout::LayoutPage;
pub use overview::OverviewPage;
pub use tabs::TabsPage;
pub use themes::ThemeShowcase;

mod code_cards;
pub use code_cards::CodeCardsPage;

mod tables;
pub use tables::TablesPage;

mod documents;
pub use documents::DocumentsPage;

mod commands;
pub use commands::CommandsPage;

mod masonry;
pub use masonry::MasonryPage;

#[cfg(all(feature = "video", any(target_os = "macos", target_os = "windows")))]
mod video;
#[cfg(all(feature = "video", any(target_os = "macos", target_os = "windows")))]
pub use video::VideoPage;

mod media;
pub use media::MediaPage;

mod forms;
pub use forms::FormsPage;
