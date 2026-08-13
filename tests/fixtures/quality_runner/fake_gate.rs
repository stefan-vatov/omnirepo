use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    process::{self, Command},
    thread,
    time::Duration,
};

fn argument(name: &str) -> String {
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == name {
            return arguments
                .next()
                .unwrap_or_else(|| panic!("missing value for {name}"));
        }
    }
    panic!("missing argument {name}");
}

fn usize_argument(name: &str) -> usize {
    argument(name)
        .parse()
        .unwrap_or_else(|error| panic!("invalid {name}: {error}"))
}

fn has_flag(name: &str) -> bool {
    env::args().skip(1).any(|argument| argument == name)
}

fn optional_argument(name: &str) -> Option<String> {
    let mut arguments = env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == name {
            return arguments.next();
        }
    }
    None
}

fn main() {
    if let Some(marker) = optional_argument("--delayed-marker") {
        thread::sleep(Duration::from_secs(30));
        fs::write(marker, b"late").expect("write delayed marker");
        return;
    }

    let label = argument("--label");
    let exit_code = argument("--exit")
        .parse::<i32>()
        .unwrap_or_else(|error| panic!("invalid --exit: {error}"));
    let stdout_bytes = usize_argument("--stdout-bytes");
    let stderr_bytes = usize_argument("--stderr-bytes");
    let log_path = env::var("QUALITY_FIXTURE_LOG").expect("QUALITY_FIXTURE_LOG is set");
    let env_marker = env::var("QUALITY_FIXTURE_ENV").expect("QUALITY_FIXTURE_ENV is set");

    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .expect("open fake gate log");
    let current_dir = env::current_dir().expect("read fake gate working directory");
    writeln!(log, "{label}|{}|{env_marker}", current_dir.display())
        .expect("write fake gate execution record");

    if has_flag("--require-eof") {
        let mut input = String::new();
        let read = io::stdin()
            .read_to_string(&mut input)
            .expect("read fake gate stdin");
        assert_eq!(read, 0, "the quality runner must close gate stdin");
    }

    if has_flag("--hang") {
        let marker = env::var("QUALITY_FIXTURE_CHILD_MARKER")
            .expect("QUALITY_FIXTURE_CHILD_MARKER is set");
        let current_executable = env::current_exe().expect("resolve fake gate executable");
        Command::new(current_executable)
            .args(["--delayed-marker", &marker])
            .spawn()
            .expect("spawn fake gate descendant");
        loop {
            thread::park_timeout(Duration::from_secs(1));
        }
    }

    io::stdout()
        .write_all("o".repeat(stdout_bytes).as_bytes())
        .expect("write fake gate stdout");
    io::stdout().flush().expect("flush fake gate stdout");
    io::stderr()
        .write_all("e".repeat(stderr_bytes).as_bytes())
        .expect("write fake gate stderr");
    io::stderr().flush().expect("flush fake gate stderr");
    process::exit(exit_code);
}
