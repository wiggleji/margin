//! Interface and code fonts (Settings → Appearance).
//!
//! The installed families are listed on a background thread the first time
//! Settings asks (scanning can take a few hundred ms). A choice is saved with
//! its file path, so starting the app loads that file directly without a scan.

use eframe::egui;
use margin_core::config::FontChoice;
use std::sync::{Arc, Mutex, OnceLock};

/// An installed font family.
pub struct Family {
    pub name: String,
    /// Fixed pitch (a code font).
    pub mono: bool,
    pub choice: FontChoice,
}

static LIST: OnceLock<Mutex<Option<Arc<Vec<Family>>>>> = OnceLock::new();

/// The installed families, sorted by name; `None` while still scanning.
pub fn families(ctx: &egui::Context) -> Option<Arc<Vec<Family>>> {
    let mut started = false;
    let cell = LIST.get_or_init(|| {
        started = true;
        Mutex::new(None)
    });
    if started {
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let list = scan();
            *LIST.get().expect("initialised").lock().unwrap() = Some(Arc::new(list));
            ctx.request_repaint();
        });
    }
    cell.lock().unwrap().clone()
}

fn scan() -> Vec<Family> {
    use fontique::{Collection, CollectionOptions, SourceKind};
    let mut col = Collection::new(CollectionOptions { shared: false, system_fonts: true });
    let names: Vec<String> = col.family_names().map(str::to_string).collect();
    let mut out = vec![];
    for name in names {
        // hidden system families (".SF NS…") and emoji fonts aren't meant to be picked
        if name.starts_with('.') || name.contains("Emoji") || out.iter().any(|f: &Family| f.name == name) {
            continue;
        }
        let Some(fam) = col.family_by_name(&name) else { continue };
        let Some(font) = fam.default_font() else { continue };
        let SourceKind::Path(path) = &font.source().kind else { continue };
        let mono = font.load(None).is_some_and(|b| is_fixed_pitch(b.as_ref(), font.index()));
        out.push(Family { name: name.clone(), mono, choice: FontChoice { family: name, path: path.to_string_lossy().into_owned(), index: font.index() } });
    }
    out.sort_by_key(|f| f.name.to_lowercase());
    out
}

fn is_fixed_pitch(data: &[u8], index: u32) -> bool {
    use skrifa::raw::TableProvider;
    skrifa::FontRef::from_index(data, index).ok().and_then(|f| f.post().ok()).is_some_and(|p| p.is_fixed_pitch() != 0)
}

/// A font file, memory-mapped (face `index` of a collection).
fn load(path: &std::path::Path, index: u32) -> Option<egui::FontData> {
    let file = std::fs::File::open(path).ok()?;
    // SAFETY: font files aren't modified while the app runs; if one were, the
    // worst case is wrong glyphs (the same trade-off egui's system fonts make).
    let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    Some(egui::FontData::from_blob(Arc::new(map), index))
}

/// The system's colour emoji font (Apple Color Emoji, Segoe UI Emoji, Noto
/// Color Emoji), loaded once.
fn color_emoji() -> Option<egui::FontData> {
    static FONT: OnceLock<Option<egui::FontData>> = OnceLock::new();
    FONT.get_or_init(|| {
        const PATHS: &[&str] = &[
            "/System/Library/Fonts/Apple Color Emoji.ttc",
            "C:\\Windows\\Fonts\\seguiemj.ttf",
            "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
            "/usr/share/fonts/noto/NotoColorEmoji.ttf",
            "/usr/share/fonts/google-noto-emoji/NotoColorEmoji.ttf",
            "/usr/share/fonts/noto-emoji/NotoColorEmoji.ttf",
        ];
        PATHS.iter().find_map(|p| load(std::path::Path::new(p), 0))
    })
    .clone()
}

/// Use these fonts (None: the built-in ones), then the system's colour
/// emoji font, so emoji (🎉 👍 ❤️ ♉ flags) look as they do everywhere else on
/// the system. Text symbols the interface font lacks (↗ ↩ ↔) come from the
/// built-in code font first, so they stay text instead of turning into emoji.
/// Anything else missing (Korean in a Latin font, ⌘) comes from egui's system
/// font fallback.
pub fn apply(ctx: &egui::Context, ui: Option<&FontChoice>, code: Option<&FontChoice>) {
    use egui::FontFamily::{Monospace, Proportional};
    let mut defs = egui::FontDefinitions::default();
    for (choice, family, key) in [(ui, Proportional, "user-ui"), (code, Monospace, "user-code")] {
        let Some(c) = choice else { continue };
        let Some(data) = load(std::path::Path::new(&c.path), c.index) else { continue };
        defs.font_data.insert(key.into(), Arc::new(data));
        defs.families.entry(family).or_default().insert(0, key.into());
    }
    defs.families.entry(Proportional).or_default().push("Hack".into());
    if let Some(e) = color_emoji() {
        defs.font_data.insert("color-emoji".into(), Arc::new(e));
        for f in [Proportional, Monospace] {
            defs.families.entry(f).or_default().push("color-emoji".into());
        }
    }
    ctx.set_fonts(defs);
}

#[cfg(test)]
mod tests {
    use eframe::egui::{FontDefinitions, FontFamily};
    use skrifa::MetadataProvider;

    /// Symbols used in the UI that the interface font lacks but emoji fonts
    /// have must come from a text font in the chain before the emoji font.
    #[test]
    fn ui_symbols_stay_text() {
        let defs = FontDefinitions::default();
        let has = |name: &str, c: char| {
            let d = &defs.font_data[name];
            skrifa::FontRef::from_index(d.font.as_ref().as_ref(), d.index).unwrap().charmap().map(c).is_some()
        };
        // the interface family before the emoji font: its fonts, then Hack (added by `apply`)
        let ui_fonts = &defs.families[&FontFamily::Proportional];
        for c in ['↗', '↩', '↔', '↑', '↓', '▸', '▾', '•'] {
            assert!(ui_fonts.iter().any(|f| has(f, c)) || has("Hack", c), "{c} has no text glyph");
        }
    }
}
