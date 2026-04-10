pub const WIDTH: usize = 1400;
pub const HEIGHT: usize = 600;

pub const SPECTRUM_HEIGHT: usize = 180;
pub const SEPARATOR_HEIGHT: usize = 8;
pub const SPECTRUM_LEFT_PAD: usize = 48;
pub const SPECTRUM_RIGHT_PAD: usize = 12;
pub const SPECTRUM_TOP_PAD: usize = 16;
pub const SPECTRUM_BOTTOM_PAD: usize = 40;
pub const SPECTRUM_PLOT_Y0: usize = SPECTRUM_TOP_PAD;
pub const SPECTRUM_PLOT_Y1: usize = SPECTRUM_HEIGHT - SPECTRUM_BOTTOM_PAD;
pub const SPECTRUM_PLOT_HEIGHT: usize = SPECTRUM_PLOT_Y1 - SPECTRUM_PLOT_Y0;
pub const SPECTRUM_DB_MIN: f32 = -120.0;
pub const SPECTRUM_DB_MAX: f32 = 0.0;
pub const SPECTRUM_SMOOTHING_ALPHA: f32 = 0.25;

pub const WATERFALL_TOP: usize = SPECTRUM_HEIGHT + SEPARATOR_HEIGHT;

pub const FREQ_WIDGET_Y: usize = 28;

pub const OM_STRIP_HEIGHT: usize = 6;
pub const OM_STRIP_Y0: usize = SPECTRUM_PLOT_Y1 + 3;
pub const OM_STRIP_Y1: usize = OM_STRIP_Y0 + OM_STRIP_HEIGHT;

pub const BAND_STRIP_HEIGHT: usize = 22;
pub const BAND_STRIP_Y0: usize = OM_STRIP_Y1 + 4;
pub const BAND_STRIP_Y1: usize = BAND_STRIP_Y0 + BAND_STRIP_HEIGHT;

pub const CONTROL_PANEL_WIDTH: usize = 56;

pub const MAIN_CONTENT_X0: usize = LEFT_PANE_WIDTH;
pub const MAIN_CONTENT_WIDTH: usize = WIDTH - LEFT_PANE_WIDTH;

pub const PLOT_WIDTH: usize = MAIN_CONTENT_WIDTH - CONTROL_PANEL_WIDTH;

pub const CONTROL_PANEL_X0: usize = WIDTH - CONTROL_PANEL_WIDTH;
pub const CONTROL_PANEL_X1: usize = WIDTH;

pub const ZOOM_SLIDER_X0: usize = CONTROL_PANEL_X0 + 18;
pub const ZOOM_SLIDER_X1: usize = CONTROL_PANEL_X0 + 38;
pub const ZOOM_SLIDER_Y0: usize = 40;
pub const ZOOM_SLIDER_Y1: usize = HEIGHT - 40;

pub const SPECTRUM_PLOT_X0: usize = MAIN_CONTENT_X0 + SPECTRUM_LEFT_PAD;
pub const SPECTRUM_PLOT_X1: usize = CONTROL_PANEL_X0 - SPECTRUM_RIGHT_PAD;
pub const SPECTRUM_PLOT_WIDTH: usize = SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0;

pub const FREQ_WIDGET_X: usize = MAIN_CONTENT_X0 + 70;

pub const LEFT_PANE_WIDTH: usize = 260;
pub const PANEL_PADDING: usize = 10;
pub const HEADER_HEIGHT: usize = 28;
pub const ROW_HEIGHT: usize = 26;
pub const BUTTON_HEIGHT: usize = 30;
pub const FIELD_HEIGHT: usize = 26;
pub const SECTION_SPACING: usize = 10;

pub const LEFT_GUTTER: f32 = 64.0;
pub const RIGHT_GUTTER: f32 = 12.0;
pub const TOP_GUTTER: f32 = 6.0;
pub const BOTTOM_GUTTER: f32 = 34.0;

pub const WATERFALL_IMAGE_WIDTH: usize = SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0;
pub const WATERFALL_IMAGE_HEIGHT: usize = HEIGHT - WATERFALL_TOP;
