use std::{
    process::ExitCode,
    sync::{Arc, Condvar, Mutex},
};

use argh::FromArgs;
use fft_convolver::FFTConvolver;
use jack::NotificationHandler;

const CLIENT_NAME: &str = "sjconv";

/// A simple standalone convolver for JACK.
#[derive(FromArgs)]
struct Args {
    /// path to the impulse response
    #[argh(option, short = 'f')]
    file: String,

    /// number of input/output channels (default: 2)
    #[argh(option, short = 'p', default = "2")]
    ports: u32,
}

struct State {
    ir: Box<[f32]>,
    channels: Box<[ChannelState]>,
}

struct ChannelState {
    input: jack::Port<jack::AudioIn>,
    output: jack::Port<jack::AudioOut>,
    convolver: FFTConvolver<f32>,
}

struct Notifications(Arc<(Mutex<bool>, Condvar)>);

impl NotificationHandler for Notifications {
    unsafe fn shutdown(&mut self, _status: jack::ClientStatus, _reason: &str) {
        let mut exit = self.0.0.lock().unwrap();
        *exit = true;
        self.0.1.notify_one();
    }
}

fn main() -> ExitCode {
    match run() {
        Err(e) => {
            log::error!("{e}");
            ExitCode::FAILURE
        }
        _ => ExitCode::SUCCESS,
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    log::set_logger(Box::leak(Box::new(Logger)))
        .map_err(|e| format!("Failed to set logger: {e}"))?;
    log::set_max_level(log::LevelFilter::Trace);

    let args = argh::from_env::<Args>();

    log::info!("sjconv v{}", env!("CARGO_PKG_VERSION"));

    if args.ports == 0 {
        return Err("Number of ports must be more than 0".into());
    }

    let reader = hound::WavReader::open(&args.file)
        .map_err(|e| format!("Couldn't load {}: {e}", args.file))?;

    let spec = reader.spec();

    if spec.channels != 1 {
        return Err("Impulse response must have only 1 channel".into());
    }

    let samples = match spec.sample_format {
        hound::SampleFormat::Float => {
            let sample_reader = reader.into_samples::<f32>();
            sample_reader.map(|s| s.unwrap()).collect()
        }
        hound::SampleFormat::Int => {
            let sample_reader = reader.into_samples::<i32>();
            let divisor = (1u32 << (spec.bits_per_sample as u32 - 1)) as f64;
            sample_reader
                .map(|s| (s.unwrap() as f64 / divisor) as f32)
                .collect()
        }
    };

    let (client, _) = jack::Client::new(CLIENT_NAME, jack::ClientOptions::default())
        .map_err(|e| format!("Couldn't create JACK client: {e}"))?;

    if spec.sample_rate != client.sample_rate() {
        return Err(
            "Sample rate of the inpulse response must match the sample rate of the JACK server"
                .into(),
        );
    }

    let channels = (1..=args.ports)
        .map(|i| {
            let input = client
                .register_port(&format!("Input.{i}"), jack::AudioIn::default())
                .map_err(|e| format!("Couldn't register input port {i}: {e}"))?;

            let output = client
                .register_port(&format!("Output.{i}"), jack::AudioOut::default())
                .map_err(|e| format!("Couldn't register output port {i}: {e}"))?;

            let convolver = FFTConvolver::default();

            Ok(ChannelState {
                input,
                output,
                convolver,
            })
        })
        .collect::<Result<_, String>>()?;

    let process_handler = jack::contrib::ClosureProcessHandler::with_state(
        State {
            ir: samples,
            channels,
        },
        process_callback,
        buffer_callback,
    );

    let notification = Arc::new((Mutex::new(false), Condvar::new()));
    let _active_client = client
        .activate_async(Notifications(notification.clone()), process_handler)
        .map_err(|e| format!("Couldn't activate the client: {e}"))?;

    log::info!(
        "Loaded impulse response: \
        \n\tSample rate: {} \
        \n\tSample format: {:?} \
        \n\tBits per sample: {} \
        \n\tChannels: {}",
        spec.sample_rate,
        spec.sample_format,
        spec.bits_per_sample,
        spec.channels
    );
    log::info!("Processing started");

    let mut exit = notification.0.lock().unwrap();
    while !*exit {
        exit = notification.1.wait(exit).unwrap();
    }

    log::info!("JACK has shutdown, exiting");
    Ok(())
}

fn process_callback(state: &mut State, _: &jack::Client, ps: &jack::ProcessScope) -> jack::Control {
    for channel in &mut state.channels {
        let _ = channel
            .convolver
            .process(channel.input.as_slice(ps), channel.output.as_mut_slice(ps));
    }
    jack::Control::Continue
}

fn buffer_callback(state: &mut State, _: &jack::Client, frames: jack::Frames) -> jack::Control {
    for channel in &mut state.channels {
        if let Err(e) = channel.convolver.init(frames as usize, &state.ir) {
            log::error!("Couldn't initialize convolver: {e}");
            return jack::Control::Quit;
        }
    }
    jack::Control::Continue
}

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        match record.level() {
            log::Level::Error => eprintln!("[{}] {}", record.level(), record.args()),
            _ => println!("[{}] {}", record.level(), record.args()),
        }
    }

    fn flush(&self) {}
}
