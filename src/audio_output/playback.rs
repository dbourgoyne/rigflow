use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub struct AudioPlayback {
    queue: Arc<Mutex<VecDeque<f32>>>,
    _stream: cpal::Stream,
}

impl AudioPlayback {
    pub fn new(sample_rate_hz: u32) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No default audio output device found".to_string())?;

        let supported_configs = device
            .supported_output_configs()
            .map_err(|e| format!("Failed to query output configs: {e}"))?;

        let mut chosen = None;

        for cfg_range in supported_configs {
            if cfg_range.channels() == 1 && cfg_range.sample_format() == cpal::SampleFormat::F32 {
                let min = cfg_range.min_sample_rate().0;
                let max = cfg_range.max_sample_rate().0;
                if sample_rate_hz >= min && sample_rate_hz <= max {
                    chosen = Some(cfg_range.with_sample_rate(cpal::SampleRate(sample_rate_hz)));
                    break;
                }
            }
        }

        let config = if let Some(cfg) = chosen {
            cfg.config()
        } else {
            let default_config = device
                .default_output_config()
                .map_err(|e| format!("Failed to get default output config: {e}"))?;

            let mut cfg: cpal::StreamConfig = default_config.config();
            cfg.channels = 1;
            cfg.sample_rate = cpal::SampleRate(sample_rate_hz);
            cfg
        };

        let queue = Arc::new(Mutex::new(VecDeque::<f32>::new()));
        let queue_for_cb = Arc::clone(&queue);

        let err_fn = |err| eprintln!("Audio playback error: {err}");

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    let mut q = queue_for_cb.lock().unwrap();
                    for sample in data.iter_mut() {
                        *sample = q.pop_front().unwrap_or(0.0);
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("Failed to build output stream: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start output stream: {e}"))?;

        Ok(Self {
            queue,
            _stream: stream,
        })
    }

    pub fn push_samples(&self, samples: &[f32]) {
        let mut q = self.queue.lock().unwrap();
        q.extend(samples.iter().copied());
    }

    pub fn queued_samples(&self) -> usize {
        self.queue.lock().unwrap().len()
    }
}
