mod configuration;
mod lifecycle;
mod managed_content;
mod platform;
mod repository;
mod source;

fn main() {
    let command = configuration::parse();
    std::process::exit(lifecycle::run_invocation(command) as i32);
}
