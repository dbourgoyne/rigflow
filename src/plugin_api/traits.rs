use crate::events::ServerEvent;

pub trait RadioPlugin {
    fn name(&self) -> &'static str;
    fn on_event(&mut self, event: ServerEvent);
}
