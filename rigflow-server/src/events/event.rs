#[derive(Clone, Debug)]
pub enum ServerEvent {
    WaterfallFrame(Vec<u8>),
    AudioFrame(Vec<f32>),
    Tick,
}
