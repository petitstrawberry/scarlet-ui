mod build;
mod cli;
#[cfg(feature = "gpu")]
mod gpu;
mod native;
mod rpc;
mod server;
mod watch;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.get(1).and_then(|arg| arg.to_str()) == Some("serve") {
        args.remove(1);
        let serve_args = match cli::ServeArgs::try_parse_from(args) {
            Ok(args) => args,
            Err(error) => {
                error.exit();
            }
        };
        return server::serve(serve_args);
    }

    let run_args = match cli::RunArgs::try_parse() {
        Ok(args) => args,
        Err(error) => {
            error.exit();
        }
    };

    match native::run(run_args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[preview] {error}");
            ExitCode::from(1)
        }
    }
}
