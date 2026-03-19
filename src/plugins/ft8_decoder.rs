use crate::plugin_api::traits::RadioPlugin;
use crate::events::ServerEvent;

pub struct FT8Decoder;

impl RadioPlugin for FT8Decoder {
fn name(&self) -> &'static str {
"FT8 Decoder"
}

fn on_event(&mut self, event: ServerEvent) {
    if let ServerEvent::AudioFrame(_) = event {
        println!("FT8 plugin received audio frame");
    }
}

}
