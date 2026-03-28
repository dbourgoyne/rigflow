use crate::{
    app::{
        layout::{OM_STRIP_Y0, OM_STRIP_Y1, SPECTRUM_PLOT_X0, SPECTRUM_PLOT_X1},
        om_bands::{om_segments_for_license, LicenseClass, OmKind},
        state::UiState,
    },
    render::{
        color::{
            COLOR_OM_CW_ONLY, COLOR_OM_FIXED_DIGITAL, COLOR_OM_PHONE_IMAGE,
            COLOR_OM_RTTY_DATA, COLOR_OM_SSB_PHONE, COLOR_OM_USB_PHONE_CW_RTTY_DATA,
        },
        text::draw_text,
    },
};
use crate::app::frequency_view::{freq_to_plot_x, visible_left_hz, visible_right_hz, visible_span_hz};

pub fn draw_om_band_strip(
    buffer: &mut [u32],
    fb_width: usize,
    state: &UiState,
) {
    if state.input_sample_rate_hz <= 0.0 {
        return;
    }

    let left_hz = visible_left_hz(state);
    let right_hz = visible_right_hz(state);

    let segments = om_segments_for_license(state.selected_license);

    let mut any_visible = false;

    for seg in segments {
        let visible_start_hz = left_hz.max(seg.start_hz);
        let visible_end_hz = right_hz.min(seg.end_hz);

        if visible_start_hz >= visible_end_hz {
            continue;
        }

        let Some(mut x0) = freq_to_plot_x(visible_start_hz, state) else {
            continue;
        };
        let Some(mut x1) = freq_to_plot_x(visible_end_hz, state) else {
            continue;
        };

        if x0 > x1 {
            std::mem::swap(&mut x0, &mut x1);
        }

        x0 = x0.max(SPECTRUM_PLOT_X0);
        x1 = x1.min(SPECTRUM_PLOT_X1.saturating_sub(1));

        if x0 >= x1 {
            continue;
        }

        any_visible = true;

        let color = om_kind_color(seg.kind);

        for y in OM_STRIP_Y0..OM_STRIP_Y1 {
            let row = y * fb_width;
            for x in x0..=x1 {
                buffer[row + x] = color;
            }
        }
    }

    if any_visible {
        draw_license_label(buffer, fb_width, state.selected_license);
    }
}

fn draw_license_label(
    buffer: &mut [u32],
    fb_width: usize,
    license: LicenseClass,
) {
    let label = license_name(license);

    let text_width = label.len() * 6;
    let right_margin = 4usize;
    let x = SPECTRUM_PLOT_X1
        .saturating_sub(right_margin)
        .saturating_sub(text_width);
    let y = OM_STRIP_Y0.saturating_sub(1);

    draw_text(buffer, fb_width, x, y, label, 0x00f0f0f0);
}

fn license_name(license: LicenseClass) -> &'static str {
    match license {
        LicenseClass::AmateurExtra => "Amateur Extra",
        LicenseClass::Advanced => "Advanced",
        LicenseClass::General => "General",
        LicenseClass::Technician => "Technician",
        LicenseClass::Novice => "Novice",
    }
}

fn om_kind_color(kind: OmKind) -> u32 {
    match kind {
        OmKind::RttyData => COLOR_OM_RTTY_DATA,
        OmKind::PhoneImage => COLOR_OM_PHONE_IMAGE,
        OmKind::CwOnly => COLOR_OM_CW_ONLY,
        OmKind::SsbPhone => COLOR_OM_SSB_PHONE,
        OmKind::UsbPhoneCwRttyData => COLOR_OM_USB_PHONE_CW_RTTY_DATA,
        OmKind::FixedDigitalMessages => COLOR_OM_FIXED_DIGITAL,
    }
}
