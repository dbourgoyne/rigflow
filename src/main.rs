use tokio::time::{sleep, Duration};

use radio_server::events::{EventBus, ServerEvent};
use radio_server::dsp::fft::fake_fft;
use radio_server::waterfall::generator::generate_frame;
use radio_server::streaming::audio_stream::generate_audio_frame;

#[tokio::main]
async fn main() {
println!("Radio server starting...");

let bus = EventBus::new(100);
let mut sub = bus.subscribe();

tokio::spawn(async move {
    while let Ok(event) = sub.recv().await {
        match event {
            ServerEvent::WaterfallFrame(_) => println!("Waterfall frame received"),
            ServerEvent::AudioFrame(_) => println!("Audio frame received"),
            ServerEvent::Tick => {}
        }
    }
});

loop {
    let fft = fake_fft(512);
    let wf = generate_frame(&fft);
    let audio = generate_audio_frame(256);

    bus.publish(ServerEvent::WaterfallFrame(wf));
    bus.publish(ServerEvent::AudioFrame(audio));

    sleep(Duration::from_millis(500)).await;
}

}
