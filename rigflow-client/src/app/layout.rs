pub const WIDTH: usize = 1024;
pub const HEIGHT: usize = 600;

pub const SPECTRUM_HEIGHT: usize = 180;
pub const SEPARATOR_HEIGHT: usize = 8;
pub const SPECTRUM_LEFT_PAD: usize = 48;
pub const SPECTRUM_RIGHT_PAD: usize = 12;
pub const SPECTRUM_TOP_PAD: usize = 16;
pub const SPECTRUM_BOTTOM_PAD: usize = 18;
pub const SPECTRUM_PLOT_X0: usize = SPECTRUM_LEFT_PAD;
pub const SPECTRUM_PLOT_X1: usize = WIDTH - SPECTRUM_RIGHT_PAD;
pub const SPECTRUM_PLOT_WIDTH: usize = SPECTRUM_PLOT_X1 - SPECTRUM_PLOT_X0;
pub const SPECTRUM_PLOT_Y0: usize = SPECTRUM_TOP_PAD;
pub const SPECTRUM_PLOT_Y1: usize = SPECTRUM_HEIGHT - SPECTRUM_BOTTOM_PAD;
pub const SPECTRUM_PLOT_HEIGHT: usize = SPECTRUM_PLOT_Y1 - SPECTRUM_PLOT_Y0;
pub const SPECTRUM_DB_MIN: f32 = -120.0;
pub const SPECTRUM_DB_MAX: f32 = 0.0;
pub const SPECTRUM_SMOOTHING_ALPHA: f32 = 0.25;

pub const WATERFALL_TOP: usize = SPECTRUM_HEIGHT + SEPARATOR_HEIGHT;
