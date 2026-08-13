pub(crate) use crate::platform::authority::*;
use crate::platform::authority::{
    ObjectClass, PathError,
    backend::{map_io, open_at},
};

const EACCES: i32 = 13;
#[cfg(target_os = "linux")]
const ELOOP: i32 = 40;
#[cfg(target_os = "macos")]
const ELOOP: i32 = 62;
const ENOENT: i32 = 2;
const ENOTDIR: i32 = 20;
const EISDIR: i32 = 21;
const EPERM: i32 = 1;

mod adapters;
mod backend;
pub(crate) mod capability;
mod paths;
mod revalidation;
mod unix_failures;
