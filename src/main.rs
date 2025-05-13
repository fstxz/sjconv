use std::process::ExitCode;

use clap::Parser;
use fft_convolver::FFTConvolver;
use jack::NotificationHandler;

const CLIENT_NAME: &str = "sjconv";

#[derive(Parser)]
#[command(version)]
struct Args {
    /// Path to the impulse response
    #[arg(short, long, value_name = "file.wav")]
    file: String,

    /// Number of input/output channels
    #[arg(short, long, default_value_t = 2)]
    ports: u32,
}

struct State {
    inputs: Vec<jack::Port<jack::AudioIn>>,
    outputs: Vec<jack::Port<jack::AudioOut>>,
    convolvers: Vec<FFTConvolver<f32>>,
}

struct Notifications;

impl NotificationHandler for Notifications {
    unsafe fn shutdown(&mut self, _status: jack::ClientStatus, _reason: &str) {
        // TODO: exit more gracefully once https://github.com/RustAudio/rust-jack/issues/219 is resolved
        std::process::exit(0)
    }
}

fn main() -> ExitCode {
    let args = Args::parse();

    let Ok(reader) = hound::WavReader::open(&args.file)
        .map_err(|e| eprintln!("Couldn't load {}: {e}", args.file))
    else {
        return ExitCode::FAILURE;
    };

    let spec = reader.spec();
    println!("File loaded: {:?}", spec);

    if spec.channels != 1 {
        eprintln!("Impulse response must have only 1 channel");
        return ExitCode::FAILURE;
    }

    let samples = match spec.sample_format {
        hound::SampleFormat::Float => {
            let sample_reader = reader.into_samples::<f32>();
            sample_reader.map(|s| s.unwrap()).collect::<Vec<_>>()
        }
        hound::SampleFormat::Int => {
            let sample_reader = reader.into_samples::<i32>();
            sample_reader
                .map(|s| {
                    (s.unwrap() as f64 / (2u32.pow(spec.bits_per_sample as u32 - 1)) as f64) as f32
                })
                .collect::<Vec<_>>()
        }
    };

    let Ok((client, _)) = jack::Client::new(CLIENT_NAME, jack::ClientOptions::default())
        .map_err(|e| eprintln!("Couldn't create JACK client: {e}"))
    else {
        return ExitCode::FAILURE;
    };

    if spec.sample_rate as usize != client.sample_rate() {
        eprintln!(
            "Sample rate of the inpulse response must match the sample rate of the JACK server"
        );
        return ExitCode::FAILURE;
    }

    let (inputs, outputs, convolvers) = (1..=args.ports)
        .map(|i| {
            let input = client
                .register_port(&format!("Input.{i}"), jack::AudioIn::default())
                .unwrap();

            let output = client
                .register_port(&format!("Output.{i}"), jack::AudioOut::default())
                .unwrap();

            let mut convolver = FFTConvolver::default();
            convolver
                .init(client.buffer_size() as usize, &samples)
                .unwrap();

            (input, output, convolver)
        })
        .collect::<(Vec<_>, Vec<_>, Vec<_>)>();

    let process_handler = jack::contrib::ClosureProcessHandler::with_state(
        State {
            inputs,
            outputs,
            convolvers,
        },
        |state, _, ps| -> jack::Control {
            for ((input, output), convolver) in std::iter::zip(&state.inputs, &mut state.outputs)
                .map(|(i, o)| (i.as_slice(ps), o.as_mut_slice(ps)))
                .zip(&mut state.convolvers)
            {
                let _ = convolver.process(input, output);
            }

            jack::Control::Continue
        },
        move |_, _, _| jack::Control::Continue,
    );

    let Ok(_active_client) = client
        .activate_async(Notifications, process_handler)
        .map_err(|e| eprintln!("Couldn't activate the client: {e}"))
    else {
        return ExitCode::FAILURE;
    };

    println!("Started");

    loop {
        std::thread::park();
    }
}
