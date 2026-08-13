use std::process::ExitCode;

fn main() -> ExitCode {
    let output = omnirepo_dev::run(std::env::args().skip(1));
    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    ExitCode::from(output.status)
}
