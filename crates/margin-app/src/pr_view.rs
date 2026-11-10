//! GitHub PR card for a branch: merge readiness, CI, comments. Display-only;
//! everything links out to GitHub.

use crate::app::{Action, RepoView};
use crate::markdown_view::{self, MdAction};
use crate::theme;
use eframe::egui::{self, RichText, Ui};
use margin_core::github::{CiState, CommentKind, PrSummary, Tone};

pub fn tone_color(t: Tone) -> egui::Color32 {
    match t {
        Tone::Good => theme::ADDED,
        Tone::Warn => theme::ACCENT,
        Tone::Bad => theme::ERROR,
        Tone::Neutral => theme::TEXT_DIM,
    }
}

pub fn hover_summary(pr: &PrSummary) -> String {
    format!(
        "#{} {}\n{} · {} · {} comments",
        pr.number,
        pr.title,
        pr.readiness().0,
        pr.ci_label().0,
        pr.comment_count
    )
}

pub fn card(ui: &mut Ui, rv: &mut RepoView, branch: &str, actions: &mut Vec<Action>) {
    if rv.github.is_none() {
        // Not a GitHub remote: show nothing, unless connecting failed.
        if let Some(e) = &rv.github_error {
            ui.colored_label(theme::ERROR, e);
            ui.separator();
        }
        return;
    }
    let Some(pr) = rv.prs.get(branch).cloned() else {
        ui.horizontal(|ui| {
            ui.weak("No pull request for this branch.");
            if ui.small_button("⟳").on_hover_text("Reload PR status").clicked() {
                actions.push(Action::FetchPrs { force: true });
            }
        });
        if let Some(e) = &rv.github_error {
            ui.colored_label(theme::ERROR, e);
        }
        ui.separator();
        return;
    };

    // ---- header: number/title (links out), readiness, CI, review
    ui.horizontal_wrapped(|ui| {
        let sz = ui.text_style_height(&egui::TextStyle::Body);
        ui.add(crate::icons::image(ui.ctx(), crate::icons::Icon::Github, sz, theme::TEXT));
        let link = ui
            .add(egui::Label::new(RichText::new(format!("#{} {}", pr.number, pr.title)).strong().color(ui.visuals().hyperlink_color)).sense(egui::Sense::click()))
            .on_hover_text("Open on GitHub");
        if link.clicked() {
            actions.push(Action::OpenUrl(pr.url.clone()));
        }
    });
    ui.horizontal(|ui| {
        let (ready, tone) = pr.readiness();
        badge(ui, ready, tone_color(tone));
        let (ci, tone) = pr.ci_label();
        badge(ui, ci, tone_color(tone));
        if let Some(r) = &pr.review_decision {
            let (label, tone) = match r.as_str() {
                "APPROVED" => ("Approved", Tone::Good),
                "CHANGES_REQUESTED" => ("Changes requested", Tone::Bad),
                _ => ("Review required", Tone::Warn),
            };
            if label != ready {
                badge(ui, label, tone_color(tone));
            }
        }
        if ui.small_button("⟳").on_hover_text("Reload PR status").clicked() {
            actions.push(Action::FetchPrs { force: true });
        }
    });

    // ---- details (lazy)
    match rv.pr_detail.get(branch) {
        None => {
            if rv.pr_requested.insert(branch.to_string()) {
                actions.push(Action::FetchPrDetail(branch.to_string()));
            }
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Loading checks and comments…");
            });
        }
        Some(Err(e)) => {
            ui.colored_label(theme::ERROR, e);
        }
        Some(Ok(d)) => {
            let failing = d.checks.iter().filter(|c| c.state == CiState::Failure).count();
            let running = d.checks.iter().filter(|c| c.state == CiState::Pending).count();
            if !d.checks.is_empty() {
                let title = format!(
                    "Checks  {} passed{}{}",
                    d.checks.len() - failing - running,
                    if failing > 0 { format!(" · {failing} failing") } else { String::new() },
                    if running > 0 { format!(" · {running} running") } else { String::new() },
                );
                egui::CollapsingHeader::new(RichText::new(title).small())
                    .id_salt(("pr_checks", branch))
                    .default_open(failing > 0)
                    .show(ui, |ui| {
                        for c in &d.checks {
                            ui.horizontal(|ui| {
                                let color = match c.state {
                                    CiState::Success => theme::ADDED,
                                    CiState::Failure => theme::ERROR,
                                    CiState::Pending => theme::ACCENT,
                                    CiState::None => theme::TEXT_DIM,
                                };
                                ui.label(RichText::new("●").small().color(color));
                                match &c.url {
                                    Some(u) => {
                                        if ui.link(&c.name).clicked() {
                                            actions.push(Action::OpenUrl(u.clone()));
                                        }
                                    }
                                    None => {
                                        ui.label(&c.name);
                                    }
                                }
                                ui.label(RichText::new(c.detail.to_lowercase().replace('_', " ")).small().color(theme::TEXT_DIM));
                            });
                        }
                    });
            }

            let threads = if d.unresolved_threads > 0 {
                format!(" · {} unresolved thread{}", d.unresolved_threads, if d.unresolved_threads == 1 { "" } else { "s" })
            } else {
                String::new()
            };
            egui::CollapsingHeader::new(RichText::new(format!("Comments ({}){threads}", d.comments.len())).small())
                .id_salt(("pr_comments", branch))
                .default_open(true)
                .show(ui, |ui| {
                    if d.comments.is_empty() {
                        ui.weak("No comments yet.");
                    }
                    for c in &d.comments {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&c.author).strong());
                            match &c.kind {
                                CommentKind::Comment => {}
                                CommentKind::Review { state } => {
                                    let (t, col) = match state.as_str() {
                                        "APPROVED" => ("approved", theme::ADDED),
                                        "CHANGES_REQUESTED" => ("requested changes", theme::ERROR),
                                        _ => ("reviewed", theme::TEXT_DIM),
                                    };
                                    ui.label(RichText::new(t).small().color(col));
                                }
                                CommentKind::Thread { path, line, replies } => {
                                    let loc = match line {
                                        Some(l) => format!("{path}:{l}"),
                                        None => path.clone(),
                                    };
                                    ui.label(RichText::new(loc).small().monospace().color(theme::ACCENT));
                                    if *replies > 0 {
                                        ui.label(RichText::new(format!("+{replies} replies")).small().color(theme::TEXT_DIM));
                                    }
                                }
                            }
                            ui.label(RichText::new(c.created_at.get(..16).unwrap_or(&c.created_at).replace('T', " ")).small().color(theme::TEXT_DIM));
                            if !c.url.is_empty() && ui.small_button("Open").clicked() {
                                actions.push(Action::OpenUrl(c.url.clone()));
                            }
                        });
                        let body = clean_body(&c.body);
                        if !body.is_empty() {
                            egui::Frame::new().inner_margin(egui::Margin { left: 10, ..Default::default() }).show(ui, |ui| {
                                if let Some(MdAction::Link(u)) = markdown_view::render(ui, &body) {
                                    actions.push(Action::OpenUrl(u));
                                }
                            });
                        }
                        ui.add_space(4.0);
                    }
                });
        }
    }
    ui.separator();
}

fn badge(ui: &mut Ui, text: &str, color: egui::Color32) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(3.0)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small().color(color));
        });
}

/// Strip HTML comments (PR templates) and cap length — this is a glance view.
fn clean_body(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    let out = out.trim();
    const MAX: usize = 800;
    if out.len() <= MAX {
        return out.to_string();
    }
    let mut end = MAX;
    while !out.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &out[..end])
}
