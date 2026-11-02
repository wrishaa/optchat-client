//! OptChat's UI on GPUI, shared by the macOS app and the iOS app.
#![recursion_limit = "256"]
pub mod composer;
pub mod dialogs;
pub mod feed;
pub mod fmt;
pub mod kit;
pub mod markdown;
pub mod notices_view;
pub mod panels;
pub mod phone;
pub mod statusbar;
pub mod tabs;
pub mod tasks_view;
pub mod testcmd;
pub mod theme;
pub mod tv;
pub mod ui;
pub mod workspace;
pub mod workspaces;

// the client's modules under the paths the views use
pub use optchat_client::{client, media, mic, state, tokens, LAUNCH};
/// the platform hooks (the optchat-platform crate), under the path the views and shells use
pub use optchat_platform as platform;

/// icons and fonts, embedded (assets/)
pub mod assets {
    include!(concat!(env!("OUT_DIR"), "/assets.rs"));
}
pub struct Assets;
impl gpui::AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        Ok(assets::ASSETS.iter().find(|(p, _)| *p == path).map(|(_, b)| std::borrow::Cow::Borrowed(*b)))
    }
    fn list(&self, path: &str) -> anyhow::Result<Vec<gpui::SharedString>> {
        Ok(assets::ASSETS.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| gpui::SharedString::from(*p)).collect())
    }
}
