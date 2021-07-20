use cpal::{StreamConfig};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn main() -> anyhow::Result<()> {
    // Set up an audio Device.
    let host = cpal::default_host();

    let input = host.default_input_device()
        .expect("no input device available.");
    let output = host.default_output_device()
        .expect("no output device available.");
    println!("Input device: {}", input.name()?);
    println!("Output device: {}", output.name()?);

    let config: StreamConfig = output.default_input_config()?.into();
    println!("Output config:  {:?}", config);

    // Design notes:
    //
    // Below is our vector of samples. Each line between the [ square brackets ]
    // is a loop; each dot is a sample. This is many thousands of times
    // below the number of samples we actually deal with in a buffer, but this
    // is fine for an illustration.
    //
    // [
    //  .....
    //  .....
    //  .....
    //  ...
    //    ^
    //    |
    //    +----- this is where the current recording ends, and is where the
    //           looper playback index is in this example.
    //
    //
    //
    // ]
    //
    // loop_len = 5
    // loop_count = 3
    //
    // playback:
    // sample_idx = 0..loop_len-1

    let state = State::new();
    let input_state = state.clone();
    let output_state = state.clone();

    let input_data_fn = move |data: &[f32], _: &cpal::InputCallbackInfo| {
        if !input_state.is_recording.load(Ordering::Acquire) {
            // We're not recording, save nothing.
            return;
        }
        let mut samples = input_state.samples.lock().unwrap();
        for &sample in data {
            // Keep appending samples to the Vector
            let len = input_state.total_samples.load(Ordering::Acquire);
            samples[len] = sample;
            input_state.total_samples.store(len + 1, Ordering::Release);
            if input_state.is_first_loop.load(Ordering::Acquire) {
                input_state.loop_len.store(len + 1, Ordering::Release);
            }
        }
        //println!("total_samples={}, input_loop_len={}", total_samples.load(Ordering::Acquire), input_loop_len.load(Ordering::Acquire));
    };
    let input_stream = input.build_input_stream(&config, input_data_fn, err_fn)?;

    // Setup output callback & stream.
    let mut output_idx = 0;
    let output_data_fn = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        if output_state.is_first_loop.load(Ordering::Acquire) {
            // Bail; no playback yet.
            return;
        }

        let playback_samples = output_state.samples.lock().unwrap();
        let len = output_state.loop_len.load(Ordering::Acquire);
        let loop_count = div_ceil(output_state.total_samples.load(Ordering::Acquire), len);

        for sample in data {
            // Sum up all samples at each corresponding index across loops.
            let mut sum = 0.0;
            for loop_offset in 0..(loop_count - 1) {
                let sample_idx = output_idx + len * loop_offset;
                sum += playback_samples[sample_idx];
            }
            // TODO dynamic range compression!
            *sample = sum;

            output_idx += 1;
            if output_idx >= len {
                // RESET THE LOOP PLAYBACK
                output_idx = 0;
            }
        }
        //println!("output_idx={}", output_idx);
    };
    let output_stream = output.build_output_stream(&config, output_data_fn, err_fn)?;

    output_stream.play()?;
    input_stream.play()?;

    // Simulate the user hitting RECORD on the pedal three times...
    println!("RECORDING.");
    std::thread::sleep(std::time::Duration::from_secs(3));
    // end of thread::sleep() simulates the user pressing the recording button
    // a second time, signaling the end of the FIRST loop recording.
    state.is_first_loop.store(false, Ordering::Release);
    println!("SET FIRST LOOP LENGTH.");

    std::thread::sleep(std::time::Duration::from_secs(6));
    // end of thread::sleep() simulates the user pressing the recording button
    // a THIRD time, signaling the end of recording any new loops.
    state.is_recording.store(false, Ordering::Release);
    println!("DONE RECORDING.");

    std::thread::sleep(std::time::Duration::from_secs(6));

    drop(input_stream);
    drop(output_stream);

    Ok(())
}

#[derive(Clone)]
struct State {
    samples: Arc<Mutex<Vec<f32>>>,
    loop_len: Arc<AtomicUsize>,
    total_samples: Arc<AtomicUsize>,
    is_recording: Arc<AtomicBool>,
    is_first_loop: Arc<AtomicBool>,
}

impl State {
    fn new() -> Self {
        Self {
            samples: Arc::new(Mutex::new(vec![0.0; 44100 * 100])),
            loop_len: Arc::new(AtomicUsize::new(0)),
            total_samples: Arc::new(AtomicUsize::new(0)),
            is_recording: Arc::new(AtomicBool::new(true)),
            is_first_loop: Arc::new(AtomicBool::new(true)),
        }
    }
}

fn err_fn(err: cpal::StreamError) {
    eprintln!("an error occurred on stream: {}", err);
}

#[inline]
fn div_ceil(first: usize, other: usize) -> usize {
    if (first % other) > 0 && other > 0 {
        first / other + 1
    } else {
        first / other
    }
}