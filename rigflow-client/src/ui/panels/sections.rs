//! Weighted sub-section ordering shared by the Radio Control and Source Control
//! panels.
//!
//! Every sub-section carries a **weight** (lower draws first; higher sinks to the
//! bottom), a **gated** flag (Advanced/Diagnostics are hidden behind the "Show
//! advanced & diagnostics controls" checkbox), and presentation metadata.  One
//! composer ([`RigflowApp::render_panel_sections`]) sorts a panel's sections by
//! weight, draws each (skipping gated ones when `show_advanced` is off), and draws
//! the checkbox at the very bottom iff any gated section is present.
//!
//! The per-panel section *bodies* live in their own modules
//! (`radio_control::draw_radio_section_body`, `source_control::draw_source_section_body`)
//! so each can reach that module's private draw helpers; the composer just calls
//! the right one.

use crate::UiState;
use crate::ui::app::RigflowApp;
use eframe::egui;
use rigflow_core::dsp::modes::DemodMode;

/// Which panel a section belongs to (selects the body dispatcher + id-salt prefix).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Panel {
    RadioControl,
    SourceControl,
}

impl Panel {
    fn prefix(self) -> &'static str {
        match self {
            Panel::RadioControl => "rc",
            Panel::SourceControl => "sc",
        }
    }
}

/// A left-panel sub-section.  Weight determines order; `gated` ties visibility to
/// the `show_advanced` toggle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Audio,
    Receive,
    Transmit,
    Configuration,
    Recording,
    Status,
    Advanced,
    Diagnostics,
}

impl Section {
    /// Sort key: lower draws first, higher sinks to the bottom.  Gaps leave room
    /// to insert future sections without renumbering.
    fn weight(self) -> u32 {
        match self {
            Section::Audio => 100,
            Section::Receive => 200,
            Section::Transmit => 300,
            Section::Configuration => 400,
            Section::Recording => 500,
            Section::Status => 600,
            Section::Advanced => 800,
            Section::Diagnostics => 900,
        }
    }

    /// Gated sections are only shown when "Show advanced & diagnostics controls"
    /// is on, and their presence is what makes that checkbox appear.
    fn gated(self) -> bool {
        matches!(self, Section::Advanced | Section::Diagnostics)
    }

    fn title(self) -> &'static str {
        match self {
            Section::Audio => "Audio",
            Section::Receive => "Receive",
            Section::Transmit => "Transmit",
            Section::Configuration => "Configuration",
            Section::Recording => "Recording",
            Section::Status => "Status",
            Section::Advanced => "Advanced",
            Section::Diagnostics => "Diagnostics",
        }
    }

    fn salt(self) -> &'static str {
        match self {
            Section::Audio => "audio",
            Section::Receive => "receive",
            Section::Transmit => "transmit",
            Section::Configuration => "configuration",
            Section::Recording => "recording",
            Section::Status => "status",
            Section::Advanced => "advanced",
            Section::Diagnostics => "diagnostics",
        }
    }

    fn default_open(self) -> bool {
        matches!(
            self,
            Section::Audio | Section::Receive | Section::Configuration | Section::Status
        )
    }
}

impl RigflowApp {
    /// Which sections this panel currently has content for (empty sections are
    /// never listed, so no empty headers appear).
    fn present_sections(&self, panel: Panel, snapshot: &UiState) -> Vec<Section> {
        match panel {
            Panel::RadioControl => {
                // Advanced is always present (it holds the always-available WSJT-X
                // setup button plus TX Processing for USB/LSB).  Diagnostics only
                // has content (two-tone, TX-audio meters) in USB/LSB.
                let mut v = vec![
                    Section::Audio,
                    Section::Receive,
                    Section::Transmit,
                    Section::Advanced,
                ];
                if matches!(snapshot.demod_mode, DemodMode::Usb | DemodMode::Lsb) {
                    v.push(Section::Diagnostics);
                }
                v
            }
            Panel::SourceControl => {
                let mut v = vec![Section::Configuration, Section::Recording, Section::Status];
                if snapshot.source_capabilities.supports_tx_tune_test
                    || snapshot.source_capabilities.supports_fdx
                {
                    v.push(Section::Diagnostics);
                }
                v
            }
        }
    }

    /// Draw a panel's sub-sections in weight order, gating Advanced/Diagnostics
    /// behind `show_advanced`, and append the toggle checkbox iff any gated
    /// section is present.
    pub(crate) fn render_panel_sections(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &UiState,
        panel: Panel,
        show_advanced: bool,
    ) {
        let mut sections = self.present_sections(panel, snapshot);
        sections.sort_by_key(|s| s.weight());
        let any_gated = sections.iter().any(|s| s.gated());

        for sec in sections {
            if sec.gated() && !show_advanced {
                continue;
            }
            let salt = format!("{}_{}", panel.prefix(), sec.salt());
            egui::CollapsingHeader::new(sec.title())
                .id_salt(salt)
                .default_open(sec.default_open())
                .show(ui, |ui| match panel {
                    Panel::RadioControl => self.draw_radio_section_body(ui, snapshot, sec),
                    Panel::SourceControl => self.draw_source_section_body(ui, snapshot, sec),
                });
        }

        if any_gated {
            self.draw_advanced_toggle(ui, show_advanced);
        }
    }

    /// The "Show advanced & diagnostics controls" checkbox (persisted per operator).
    fn draw_advanced_toggle(&mut self, ui: &mut egui::Ui, show_advanced: bool) {
        ui.separator();
        let mut show = show_advanced;
        if ui
            .checkbox(&mut show, "Show advanced & diagnostics controls")
            .changed()
        {
            if let Ok(mut state) = self.state.lock() {
                state.show_advanced = show;
            }
            self.save_show_advanced_to_current_operator();
        }
    }
}
