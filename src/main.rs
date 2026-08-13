mod configuration;
mod lifecycle;
mod managed_content;
mod platform;
mod repository;
mod source;

fn main() {
    let _ = configuration::run();
}
