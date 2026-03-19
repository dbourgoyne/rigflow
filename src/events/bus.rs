use tokio::sync::broadcast;
use crate::events::event::ServerEvent;

#[derive(Clone)]
pub struct EventBus {
sender: broadcast::Sender<ServerEvent>,
}

impl EventBus {
pub fn new(size: usize) -> Self {
let (sender, _) = broadcast::channel(size);
Self { sender }
}

pub fn publish(&self, event: ServerEvent) {
    let _ = self.sender.send(event);
}

pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
    self.sender.subscribe()
}

}
