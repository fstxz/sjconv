# sjconv

A simple standalone convolver for JACK. It uses [fft-convolver](https://github.com/neodsp/fft-convolver) for convolution.

## Usage
```
sjconv -f <file> [-p <ports>]

Options:
  -f, --file        path to the impulse response
  -p, --ports       number of input/output channels (default: 2)
  --help, help      display usage information
```

## Building from source

Install [rustup](https://rustup.rs/) if you haven't already, then execute the following commands:

```sh
git clone https://github.com/fstxz/sjconv.git
cd sjconv
cargo build --release
```

The `sjconv` binary will be placed in the `./target/release/` directory.

## Limitations/assumptions

* Only mono impulse responses are supported
* Sample rate of the inpulse response must match the sample rate of the JACK server

## License

This program is licenced under the [MIT License](https://github.com/fstxz/sjconv/blob/master/LICENSE.txt).
