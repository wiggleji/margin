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

/// Use these fonts (None: the built-in ones). Characters a font lacks
/// (e.g. Korean in a Latin code font) still come from the built-in fonts and
/// the system fallback.
pub fn apply(ctx: &egui::Context, ui: Option<&FontChoice>, code: Option<&FontChoice>) {
    let mut defs = egui::FontDefinitions::default();
    for (choice, family, key) in [(ui, egui::FontFamily::Proportional, "user-ui"), (code, egui::FontFamily::Monospace, "user-code")] {
        let Some(c) = choice else { continue };
        let Ok(bytes) = std::fs::read(&c.path) else { continue };
        let mut data = egui::FontData::from_owned(bytes);
        data.index = c.index;
        defs.font_data.insert(key.into(), Arc::new(data));
        defs.families.entry(family).or_default().insert(0, key.into());
    }
    ctx.set_fonts(defs);
}
