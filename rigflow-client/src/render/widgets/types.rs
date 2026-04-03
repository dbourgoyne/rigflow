#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    pub fn contains(&self, px: usize, py: usize) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MouseState {
    pub x: usize,
    pub y: usize,
    pub left_clicked: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct WidgetColors {
    pub bg: u32,
    pub border: u32,
    pub text: u32,
    pub accent: u32,
}
