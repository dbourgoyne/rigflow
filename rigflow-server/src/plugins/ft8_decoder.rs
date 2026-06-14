use crate::events::ServerEvent;
use crate::plugin_api::traits::RadioPlugin;

pub struct FT8Decoder;

impl RadioPlugin for FT8Decoder {
    fn name(&self) -> &'static str {
        "FT8 Decoder"
    }

    fn on_event(&mut self, event: ServerEvent) {
        if let ServerEvent::AudioFrame(_) = event {
            // Per-frame: trace only, and through the log framework (not stdout).
            log::trace!("FT8 plugin received audio frame");
        }
    }
}
