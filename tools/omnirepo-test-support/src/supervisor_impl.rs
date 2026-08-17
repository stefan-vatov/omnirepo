// Private Linux process supervisor implementation (included by the
// omnirepo-e2e-supervisor bin wrapper on Linux hosts only).
//
// The runner starts one supervisor per fixture target.  The supervisor is a
// child subreaper, owns the target's process tree, and remains alive until
// every adopted descendant has been observed and reaped.  It communicates
// cancellation over stdin so killing the supervisor cannot orphan a target
// before the census has completed.

use std::{
    collections::BTreeSet,
    env, fs,
    io::{self, Read, Write},
    os::unix::process::CommandExt,
    path::PathBuf,
    process::{Command, ExitCode, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

use rustix::process::{
    Pid, PidfdFlags, Signal, WaitId, WaitIdOptions, getpid, pidfd_open, pidfd_send_signal,
    set_child_subreaper, set_parent_process_death_signal, waitid,
};

const GRACE: Duration = Duration::from_millis(250);
const POLL: Duration = Duration::from_millis(5);

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ProcessIdentity {
    pid: u32,
    starttime: u64,
    ppid: u32,
    pgrp: u32,
    session: u32,
}

#[derive(Debug, Default)]
struct Status {
    target_code: Option<i32>,
    target_signal: Option<i32>,
    spawn_error: Option<String>,
    tree_terminated: bool,
    reaped: bool,
    descendants_detected: bool,
    capability_failure: Option<String>,
    termination_error: Option<String>,
    survivor_count: usize,
}

pub fn main() -> ExitCode {
    let result = run();
    match result {
        Ok(code) => ExitCode::from(code as u8),
        Err(error) => {
            let _ = write_status(&Status {
                termination_error: Some(error.to_string()),
                ..Status::default()
            });
            ExitCode::from(126)
        }
    }
}

fn run() -> io::Result<i32> {
    let (cwd, target, args) = parse_args()?;
    if !proc_capabilities_available()? {
        write_status(&Status {
            capability_failure: Some("strict Linux supervisor capability probe failed".to_owned()),
            ..Status::default()
        })?;
        return Ok(126);
    }
    set_child_subreaper(Some(getpid()))?;
    set_parent_process_death_signal(Some(Signal::KILL))?;

    let (cancel_tx, cancel_rx) = mpsc::channel();
    let _reader = thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut byte = [0_u8; 1];
        if stdin.read_exact(&mut byte).is_ok() || byte[0] != 0 {
            let _ = cancel_tx.send(());
        }
    });

    let mut command = Command::new(target);
    command
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            write_status(&Status {
                spawn_error: Some(error.to_string()),
                tree_terminated: true,
                reaped: true,
                ..Status::default()
            })?;
            return Ok(126);
        }
    };
    let target_pid = child.id();
    let mut status = Status::default();
    let mut cancelled = false;
    let target_status;

    loop {
        if cancel_rx.try_recv().is_ok() {
            cancelled = true;
            status.descendants_detected = true;
            terminate_tree(target_pid, &mut status)?;
            target_status = Some(wait_for_target(&mut child, target_pid, &mut status)?);
            wait_for_empty(supervisor_pid(), target_pid, &mut status)?;
            break;
        }
        if let Some(exit) = child.try_wait()? {
            target_status = Some(exit);
            let census = census(supervisor_pid(), target_pid)?;
            if !census.is_empty() {
                status.descendants_detected = true;
                terminate_identities(&census, Signal::TERM, &mut status)?;
                wait_for_empty(supervisor_pid(), target_pid, &mut status)?;
            }
            break;
        }
        thread::sleep(POLL);
    }

    if !cancelled && target_status.is_some() {
        let remaining = census(supervisor_pid(), target_pid)?;
        if !remaining.is_empty() {
            status.descendants_detected = true;
            terminate_identities(&remaining, Signal::TERM, &mut status)?;
            wait_for_empty(supervisor_pid(), target_pid, &mut status)?;
        }
    }
    reap_children(&mut status)?;
    let remaining = census(supervisor_pid(), target_pid)?;
    status.survivor_count = remaining.len();
    status.tree_terminated = remaining.is_empty();
    status.reaped = status.tree_terminated && status.termination_error.is_none();
    if let Some(target_status) = target_status {
        status.target_code = target_status.code();
        status.target_signal = target_status.signal();
    }
    write_status(&status)?;
    let exit = status.target_code.unwrap_or(126);
    Ok(exit)
}

fn parse_args() -> io::Result<(PathBuf, PathBuf, Vec<String>)> {
    let mut args = env::args_os().skip(1);
    let mut cwd = None;
    let mut target = None;
    let mut target_args = Vec::new();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--cwd") => cwd = args.next().map(PathBuf::from),
            Some("--target") => target = args.next().map(PathBuf::from),
            Some("--") => {
                for argument in args.by_ref() {
                    target_args.push(argument.into_string().map_err(|_| {
                        io::Error::new(io::ErrorKind::InvalidInput, "target argument is not UTF-8")
                    })?);
                }
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid supervisor arguments",
                ));
            }
        }
    }
    let cwd = cwd.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing --cwd"))?;
    let target =
        target.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing --target"))?;
    Ok((cwd, target, target_args))
}

fn supervisor_pid() -> u32 {
    getpid().as_raw_nonzero().get() as u32
}

fn proc_capabilities_available() -> io::Result<bool> {
    let proc = fs::read_dir("/proc")?;
    if proc.count() == 0 {
        return Ok(false);
    }
    let pid = Pid::from_raw(std::process::id() as i32)
        .ok_or_else(|| io::Error::other("invalid supervisor PID"))?;
    let fd = pidfd_open(pid, PidfdFlags::empty())?;
    pidfd_send_signal(&fd, Signal::CONT)?;
    Ok(true)
}

fn census(supervisor_pid: u32, target_pid: u32) -> io::Result<Vec<ProcessIdentity>> {
    let all = read_proc()?;
    let mut selected = BTreeSet::new();
    let mut frontier = vec![target_pid];
    while let Some(parent) = frontier.pop() {
        for process in all.iter().filter(|process| process.ppid == parent) {
            if selected.insert(process.clone()) {
                frontier.push(process.pid);
            }
        }
    }
    if let Some(target) = all.iter().find(|process| process.pid == target_pid) {
        selected.insert(target.clone());
    } else {
        // An exited target can have descendants adopted by the subreaper.  A
        // dedicated supervisor has no peer children, so its direct children
        // are exactly the remaining target descendants.
        for process in all.iter().filter(|process| process.ppid == supervisor_pid) {
            selected.insert(process.clone());
        }
    }
    Ok(selected.into_iter().collect())
}

fn read_proc() -> io::Result<Vec<ProcessIdentity>> {
    let mut result = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|value| value.parse().ok())
        else {
            continue;
        };
        let text = match fs::read_to_string(entry.path().join("stat")) {
            Ok(text) => text,
            Err(_) => continue,
        };
        let Some(close) = text.rfind(')') else {
            continue;
        };
        let fields = text[close + 1..].split_whitespace().collect::<Vec<_>>();
        if fields.len() <= 19 {
            continue;
        }
        let Ok(ppid) = fields[1].parse() else {
            continue;
        };
        let Ok(pgrp) = fields[2].parse() else {
            continue;
        };
        let Ok(session) = fields[3].parse() else {
            continue;
        };
        let Ok(starttime) = fields[19].parse() else {
            continue;
        };
        result.push(ProcessIdentity {
            pid,
            starttime,
            ppid,
            pgrp,
            session,
        });
    }
    Ok(result)
}

fn terminate_tree(target_pid: u32, status: &mut Status) -> io::Result<()> {
    let census = census(supervisor_pid(), target_pid)?;
    terminate_identities(&census, Signal::TERM, status)?;
    Ok(())
}

fn wait_for_target(
    child: &mut std::process::Child,
    target_pid: u32,
    status: &mut Status,
) -> io::Result<std::process::ExitStatus> {
    let deadline = Instant::now() + GRACE;
    loop {
        if let Some(exit) = child.try_wait()? {
            return Ok(exit);
        }
        if Instant::now() >= deadline {
            let remaining = census(supervisor_pid(), target_pid)?;
            terminate_identities(&remaining, Signal::KILL, status)?;
            return child.wait();
        }
        thread::sleep(POLL);
    }
}

fn terminate_identities(
    identities: &[ProcessIdentity],
    signal: Signal,
    status: &mut Status,
) -> io::Result<()> {
    let current = read_proc()?;
    for identity in identities {
        let Some(observed) = current.iter().find(|process| process.pid == identity.pid) else {
            continue;
        };
        if observed.starttime != identity.starttime {
            continue;
        }
        let Some(pid) = Pid::from_raw(identity.pid as i32) else {
            continue;
        };
        match pidfd_open(pid, PidfdFlags::empty()).and_then(|fd| pidfd_send_signal(&fd, signal)) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => status.termination_error = Some(error.to_string()),
        }
    }
    Ok(())
}

fn wait_for_empty(supervisor_pid: u32, target_pid: u32, status: &mut Status) -> io::Result<()> {
    let deadline = Instant::now() + GRACE;
    loop {
        reap_children(status)?;
        let remaining = census(supervisor_pid, target_pid)?;
        if remaining.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            terminate_identities(&remaining, Signal::KILL, status)?;
            let kill_deadline = Instant::now() + GRACE;
            loop {
                reap_children(status)?;
                let final_remaining = census(supervisor_pid, target_pid)?;
                status.survivor_count = final_remaining.len();
                if final_remaining.is_empty() {
                    return Ok(());
                }
                if Instant::now() >= kill_deadline {
                    status.termination_error =
                        Some("managed process survivors remain after SIGKILL".to_owned());
                    return Ok(());
                }
                thread::sleep(POLL);
            }
        }
        thread::sleep(POLL);
    }
}

fn reap_children(status: &mut Status) -> io::Result<()> {
    loop {
        match waitid(WaitId::All, WaitIdOptions::EXITED | WaitIdOptions::NOHANG) {
            Ok(Some(_)) => {}
            Ok(None) => return Ok(()),
            Err(error) if error == rustix::io::Errno::CHILD => return Ok(()),
            Err(error) => {
                status.termination_error = Some(error.to_string());
                return Err(error.into());
            }
        }
    }
}

fn write_status(status: &Status) -> io::Result<()> {
    let path = env::var_os("OMNIREPO_E2E_SUPERVISOR_STATUS").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "missing supervisor status path",
        )
    })?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)?;
    writeln!(file, "tree_terminated={}", status.tree_terminated)?;
    writeln!(file, "reaped={}", status.reaped)?;
    writeln!(file, "descendants_detected={}", status.descendants_detected)?;
    writeln!(file, "survivor_count={}", status.survivor_count)?;
    if let Some(code) = status.target_code {
        writeln!(file, "target_code={code}")?;
    }
    if let Some(signal) = status.target_signal {
        writeln!(file, "target_signal={signal}")?;
    }
    if let Some(error) = &status.spawn_error {
        writeln!(file, "spawn_error={error}")?;
    }
    if let Some(error) = &status.capability_failure {
        writeln!(file, "capability_failure={error}")?;
    }
    if let Some(error) = &status.termination_error {
        writeln!(file, "termination_error={error}")?;
    }
    file.sync_all()
}
