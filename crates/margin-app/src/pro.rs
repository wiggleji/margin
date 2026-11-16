//! Margin Pro UI bits: the lock hint shown in place of a Pro feature.

use crate::app::Action;
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use margin_core::license::Status;

/// Where "Buy Margin Pro" goes. Set this to your store's checkout page
/// (e.g. a Paddle / Lemon Squeezy product link) before release.
pub const PURCHASE_URL: &str = "https://margin.app/pro";
/// Help → Margin Help, and Help → Report an Issue… (placeholders until the website exists).
pub const HELP_URL: &str = "https://margin.app/help";
pub const SUPPORT_URL: &str = "https://margin.app/support";

pub fn lock_hint(ui: &mut Ui, feature: &str, status: &Status, actions: &mut Vec<Action>) {
    egui::Frame::new().fill(theme::HUNK_BG()).inner_margin(8.0).corner_radius(4.0).show(ui, |ui| {
        ui.label(RichText::new(format!("{feature} is a Margin Pro feature.")).strong());
        ui.label(RichText::new(status.describe()).small().color(theme::TEXT_DIM()));
        ui.horizontal(|ui| {
            if ui.button("Enter licence key…").clicked() {
                actions.push(Action::OpenSettings);
            }
            if ui.button("Buy Margin Pro").clicked() {
                actions.push(Action::OpenUrl(PURCHASE_URL.into()));
            }
        });
    });
    ui.separator();
}
