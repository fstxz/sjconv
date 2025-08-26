use std::sync::{Arc, Condvar, Mutex};

use argh::FromArgs;
use fft_convolver::FFTConvolver;
use jack::NotificationHandler;

const CLIENT_NAME: &str = "sjconv";

#[derive(FromArgs)]
/// A simple standalone convolver for JACK.
struct Args {
    /// path to the impulse response
    #[argh(option, short = 'f')]
    file: String,

    /// number of input/output channels (default: 2)
    #[argh(option, short = 'p', default = "2")]
    ports: u32,
}

struct State {
    inputs: Vec<jack::Port<jack::AudioIn>>,
    outputs: Vec<jack::Port<jack::AudioOut>>,
    convolvers: Vec<FFTConvolver<f32>>,
}

struct Notifications(Arc<(Mutex<bool>, Condvar)>);

impl NotificationHandler for Notifications {
    unsafe fn shutdown(&mut self, _status: jack::ClientStatus, _reason: &str) {
        let mut exit = self.0.0.lock().unwrap();
        *exit = true;
        self.0.1.notify_one();
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = argh::from_env::<Args>();

    if args.ports == 0 {
        return Err("Number of ports must be more than 0".into());
    }

    let reader = hound::WavReader::open(&args.file)
        .map_err(|e| format!("Couldn't load {}: {e}", args.file))?;

    let spec = reader.spec();
    println!("File loaded: {:?}", spec);

    if spec.channels != 1 {
        return Err("Impulse response must have only 1 channel".into());
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

    let (client, _) = jack::Client::new(CLIENT_NAME, jack::ClientOptions::default())
        .map_err(|e| format!("Couldn't create JACK client: {e}"))?;

    if spec.sample_rate as usize != client.sample_rate() {
        return Err(
            "Sample rate of the inpulse response must match the sample rate of the JACK server"
                .into(),
        );
    }

    let (inputs, outputs, convolvers) = (1..=args.ports)
        .map(|i| {
            let input = client
                .register_port(&format!("Input.{i}"), jack::AudioIn::default())
                .map_err(|e| format!("Couldn't register input port {i}: {e}"))?;

            let output = client
                .register_port(&format!("Output.{i}"), jack::AudioOut::default())
                .map_err(|e| format!("Couldn't register output port {i}: {e}"))?;

            let mut convolver = FFTConvolver::default();
            convolver
                .init(client.buffer_size() as usize, &samples)
                .map_err(|e| format!("Couldn't initialize convolver for port {i}: {e}"))?;

            Ok((input, output, convolver))
        })
        .collect::<Result<(Vec<_>, Vec<_>, Vec<_>), String>>()?;

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

    let notification = Arc::new((Mutex::new(false), Condvar::new()));
    let _active_client = client
        .activate_async(Notifications(notification.clone()), process_handler)
        .map_err(|e| format!("Couldn't activate the client: {e}"))?;

    println!("Started");

    let mut exit = notification.0.lock().unwrap();
    while !*exit {
        exit = notification.1.wait(exit).unwrap();
    }

    println!("JACK has shutdown, exiting");
    Ok(())
}
