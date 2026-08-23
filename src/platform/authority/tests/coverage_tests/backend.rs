use super::*;
use std::{
    ffi::OsString,
    fs, io,
    mem::ManuallyDrop,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::ffi::OsStringExt,
    },
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::{Builder, TempDir};

const CLOSED_DESCRIPTOR_CHILD: &str = "OMNIREPO_BACKEND_CLOSED_DESCRIPTOR_CHILD";

fn run_in_isolated_child(test_name: &str) -> bool {
    if std::env::var_os(CLOSED_DESCRIPTOR_CHILD).is_some() {
        return false;
    }
    let status = Command::new(std::env::current_exe().expect("locate authority test binary"))
        .arg(test_name)
        .arg("--exact")
        .arg("--nocapture")
        .env(CLOSED_DESCRIPTOR_CHILD, "1")
        .status()
        .expect("run isolated authority descriptor test");
    assert!(
        status.success(),
        "isolated authority test failed: {test_name}"
    );
    true
}

struct ClosedFile {
    file: ManuallyDrop<fs::File>,
}

impl ClosedFile {
    fn new(file: fs::File) -> Self {
        let file = ManuallyDrop::new(file);
        let fd = file.as_raw_fd();
        drop(unsafe { OwnedFd::from_raw_fd(fd) });
        Self { file }
    }

    fn as_file(&self) -> &fs::File {
        &self.file
    }
}

impl Drop for ClosedFile {
    fn drop(&mut self) {
        let replacement = fs::File::open("/").expect("open replacement descriptor");
        let closed = std::mem::replace(&mut self.file, ManuallyDrop::new(replacement));
        std::mem::forget(ManuallyDrop::into_inner(closed));
        unsafe { ManuallyDrop::drop(&mut self.file) };
    }
}

fn closed_file() -> ClosedFile {
    ClosedFile::new(fs::File::open("/").expect("open descriptor to close"))
}

fn supported_fixture() -> Option<(
    TempDir,
    crate::platform::authority::AuthorityRoot<DestinationRepositoryRoot, ReadOnly>,
)> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
    fs::create_dir_all(&base).expect("create backend test base");
    let fixture = Builder::new()
        .prefix("authority-backend-")
        .tempdir_in(base)
        .expect("create backend fixture");
    let root = match AuthorityRoot::<DestinationRepositoryRoot, ReadOnly>::open(fixture.path()) {
        Ok(root) => root,
        Err(PathError::UnsupportedFilesystem { .. }) => return None,
        Err(error) => panic!("supported backend fixture root failed: {error}"),
    };
    Some((fixture, root))
}

#[test]
fn open_at_rejects_nul_components_before_syscall() {
    let error = open_at(-1, b"invalid\0component", 0, 0)
        .expect_err("NUL components must be rejected before openat");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "NUL is not a path component");
}

#[test]
fn map_io_errno_classes_preserve_exact_path_errors() {
    let operation = "synthetic operation";
    let path = "synthetic/path";
    let cases = [
        (
            ELOOP,
            PathError::LinkLikeObject {
                path: path.to_owned(),
            },
        ),
        (
            ENOENT,
            PathError::NotFound {
                path: path.to_owned(),
            },
        ),
        (
            ENOTDIR,
            PathError::UnsupportedObject {
                path: path.to_owned(),
                expected: ObjectClass::Directory,
            },
        ),
        (
            EISDIR,
            PathError::UnsupportedObject {
                path: path.to_owned(),
                expected: ObjectClass::RegularFile,
            },
        ),
        (
            EACCES,
            PathError::Io {
                operation: operation.to_owned(),
                path: path.to_owned(),
                kind: "permission denied".to_owned(),
                code: Some(EACCES),
            },
        ),
        (
            EPERM,
            PathError::Io {
                operation: operation.to_owned(),
                path: path.to_owned(),
                kind: "permission denied".to_owned(),
                code: Some(EPERM),
            },
        ),
    ];

    for (code, expected) in cases {
        assert_eq!(
            map_io(operation, path, io::Error::from_raw_os_error(code)),
            expected,
            "errno {code} must preserve its exact authority classification"
        );
    }

    let unknown = io::Error::new(io::ErrorKind::UnexpectedEof, "synthetic unknown error");
    assert_eq!(
        map_io(operation, path, unknown),
        PathError::Io {
            operation: operation.to_owned(),
            path: path.to_owned(),
            kind: "synthetic unknown error".to_owned(),
            code: None,
        }
    );
}

#[test]
fn filesystem_identity_reports_closed_descriptor_failure() {
    if run_in_isolated_child(
        "platform::authority::tests::coverage_tests::backend::filesystem_identity_reports_closed_descriptor_failure",
    ) {
        return;
    }
    let file = closed_file();
    let error =
        crate::platform::authority::backend::filesystem_identity(file.as_file(), 0, "closed")
            .expect_err("closed descriptor must fail filesystem identity");
    #[cfg(target_os = "linux")]
    let expected_operation = "read mount identity";
    #[cfg(target_os = "macos")]
    let expected_operation = "read filesystem type";
    assert!(matches!(
        error,
        PathError::Io {
            operation,
            path,
            code: Some(9),
            ..
        } if operation == expected_operation && path == "closed"
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn mount_identity_reports_closed_descriptor_failure() {
    if run_in_isolated_child(
        "platform::authority::tests::coverage_tests::backend::mount_identity_reports_closed_descriptor_failure",
    ) {
        return;
    }
    let file = closed_file();
    let error =
        crate::platform::authority::backend::mount_id_for_file(file.as_file(), "closed-mount")
            .expect_err("closed descriptor must fail mount identity");
    assert!(matches!(
        error,
        PathError::Io {
            operation,
            path,
            code: Some(9),
            ..
        } if operation == "read mount identity" && path == "closed-mount"
    ));
}

#[test]
fn duplicate_reports_closed_descriptor_failure() {
    if run_in_isolated_child(
        "platform::authority::tests::coverage_tests::backend::duplicate_reports_closed_descriptor_failure",
    ) {
        return;
    }
    let file = closed_file();
    let error = crate::platform::authority::backend::duplicate(file.as_file())
        .expect_err("closed descriptor must not duplicate");
    assert_eq!(error.raw_os_error(), Some(9));
}

#[test]
fn authority_root_duplication_reports_closed_descriptor_failure() {
    if run_in_isolated_child(
        "platform::authority::tests::coverage_tests::backend::authority_root_duplication_reports_closed_descriptor_failure",
    ) {
        return;
    }
    let Some((fixture, mut root)) = supported_fixture() else {
        return;
    };
    let path = RelativePath::root();
    let fd = root.test_handle_mut().as_raw_fd();
    drop(unsafe { OwnedFd::from_raw_fd(fd) });
    let error = match root.resolve_read(&path, ObjectClass::RegularFile) {
        Ok(_) => panic!("closed authority root descriptor must not resolve"),
        Err(error) => error,
    };

    let replacement = fs::File::open(fixture.path()).expect("open replacement root descriptor");
    let closed = std::mem::replace(root.test_handle_mut(), replacement);
    std::mem::forget(closed);

    assert!(matches!(
        error,
        PathError::Io {
            operation,
            path,
            code: Some(9),
            kind,
        } if operation == "duplicate authority root handle"
            && path.is_empty()
            && !kind.is_empty()
    ));
}

#[test]
fn validate_target_identity_rejects_forged_filesystem_identity() {
    let Some((fixture, root)) = supported_fixture() else {
        return;
    };
    let target_path = fixture.path().join("target");
    fs::write(&target_path, b"target").expect("write identity target");
    let target = fs::File::open(&target_path).expect("open identity target");

    let mut forged_device = root.identity();
    forged_device.filesystem.device ^= 1;
    assert!(matches!(
        crate::platform::authority::backend::validate_target_identity(
            forged_device,
            &target,
            "forged-device",
            ObjectClass::RegularFile,
        ),
        Err(PathError::MountCrossing { path }) if path == "forged-device"
    ));

    let mut forged_kind = root.identity();
    forged_kind.filesystem.kind = match forged_kind.filesystem.kind {
        FilesystemKind::Linux => FilesystemKind::MacOsApfs,
        FilesystemKind::MacOsApfs => FilesystemKind::Linux,
    };
    assert!(matches!(
        crate::platform::authority::backend::validate_target_identity(
            forged_kind,
            &target,
            "forged-kind",
            ObjectClass::RegularFile,
        ),
        Err(PathError::MountCrossing { path }) if path == "forged-kind"
    ));

    let mut forged_mount = root.identity();
    forged_mount.filesystem.mount_id = forged_mount.filesystem.mount_id.wrapping_add(1);
    assert!(matches!(
        crate::platform::authority::backend::validate_target_identity(
            forged_mount,
            &target,
            "forged-mount",
            ObjectClass::RegularFile,
        ),
        Err(PathError::MountCrossing { path }) if path == "forged-mount"
    ));
}

#[test]
fn validate_target_identity_rejects_changed_object_class() {
    let Some((fixture, root)) = supported_fixture() else {
        return;
    };
    let target_path = fixture.path().join("regular");
    fs::write(&target_path, b"regular").expect("write regular target");
    let target = fs::File::open(&target_path).expect("open regular target");
    assert!(matches!(
        crate::platform::authority::backend::validate_target_identity(
            root.identity(),
            &target,
            "regular",
            ObjectClass::Directory,
        ),
        Err(PathError::UnsupportedObject {
            path,
            expected: ObjectClass::Directory,
        }) if path == "regular"
    ));
}

#[test]
fn absolute_components_rejects_non_utf8_component() {
    let raw = OsString::from_vec(b"/authority-\xff".to_vec());
    let path = PathBuf::from(raw);
    let error = crate::platform::authority::backend::absolute_components(&path)
        .expect_err("non-UTF-8 authority component must fail");
    assert!(matches!(
        error,
        PathError::InvalidAbsolutePath { path, reason }
            if path.contains("authority") && reason == "authority paths must be UTF-8"
    ));
}
