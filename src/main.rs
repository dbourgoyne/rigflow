use tokio::time::{sleep, Duration};

use num_complex::Complex32;

use radio_server::dsp::demod::Sideband;
use radio_server::dsp::pipeline::DspPipeline;
use radio_server::events::{EventBus, ServerEvent};

#[tokio::main]
async fn main() {
    println!("Radio server starting...");

    let bus = EventBus::new(100);

    let mut pipeline = DspPipeline::new(
        14_100_000.0,
        14_074_000.0,
        48_000.0,
        3_000.0,
        101,
        4,
    );

    pipeline.set_sideband(Sideband::Usb);

    let mut sub = bus.subscribe();

    tokio::spawn(async move {
        while let Ok(event) = sub.recv().await {
            match event {
                ServerEvent::WaterfallFrame(_) => println!("Waterfall frame received"),
                ServerEvent::AudioFrame(audio) => {
                    println!("Audio frame received: {} samples", audio.len())
                }
                ServerEvent::Tick => {}
            }
        }
    });

    loop {
        let iq_block: Vec<Complex32> = (0..1024)
            .map(|i| {
                let x = i as f32 * 0.01;
                Complex32::new(x.sin(), x.cos())
            })
            .collect();

        let audio = pipeline.process_audio(&iq_block);

        bus.publish(ServerEvent::AudioFrame(audio));

        sleep(Duration::from_millis(500)).await;

    }
}