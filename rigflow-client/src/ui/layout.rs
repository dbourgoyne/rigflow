/// Overall application window size.
pub const WIDTH: usize = 1400;
pub const HEIGHT: usize = 600;

/// Left-side control pane.
pub const LEFT_PANE_WIDTH: usize = 260;

/// Main content area begins to the right of the left pane.
pub const MAIN_CONTENT_X0: usize = LEFT_PANE_WIDTH;

/// Right-side narrow control strip used by older spectrum/waterfall layout code.
pub const CONTROL_PANEL_WIDTH: usize = 56;
pub const CONTROL_PANEL_X0: usize = WIDTH - CONTROL_PANEL_WIDTH;

/// Spectrum panel sizing.
pub const SPECTRUM_HEIGHT: usize = 180;
pub const SEPARATOR_HEIGHT: usize = 8;
pub const WATERFALL_TOP: usize = SPECTRUM_HEIGHT + SEPARATOR_HEIGHT;

/// Spectrum plot padding within the spectrum panel.
pub const SPECTRUM_LEFT_PAD: usize = 48;
pub const SPECTRUM_RIGHT_PAD: usize = 12;

/// Spectrum plot bounds in the legacy pixel-coordinate layout.
pub const SPECTRUM_PLOT_X0: usize = MAIN_CONTENT_X0 + SPECTRUM_LEFT_PAD;
pub const SPECTRUM_PLOT_X1: usize = CONTROL_PANEL_X0 - SPECTRUM_RIGHT_PAD;

/// Spectrum display scale.
pub const SPECTRUM_DB_MIN: f32 = -120.0;
pub const SPECTRUM_DB_MAX: f32 = 0.0;
pub const SPECTRUM_SMOOTHING_ALPHA: f32 = 0.25;

/// Egui spectrum gutters used by the current spectrum renderer.
pub const LEFT_GUTTER: f32 = 64.0;
pub const RIGHT_GUTTER: f32 = 12.0;
pub const TOP_GUTTER: f32 = 6.0;
pub const BOTTOM_GUTTER: f32 = 34.0;

/// CPU-side waterfall image dimensions.
pub const WATERFALL_IMAGE_WIDTH: usize = SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0;
pub const WATERFALL_IMAGE_HEIGHT: usize = HEIGHT - WATERFALL_TOP;
