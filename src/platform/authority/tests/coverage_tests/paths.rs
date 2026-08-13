// Focused authority identity, path, and error coverage.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use super::super::{
    AbsolutePath, AuthorityAdapterKind, AuthorityIdentity, FilesystemIdentity, FilesystemKind,
    ObjectClass, ObjectIdentity, PathError, RelativePath,
};

fn assert_display(case_id: &str, actual: impl std::fmt::Display, expected: &str) {
    assert_eq!(
        actual.to_string(),
        expected,
        "{case_id}: display text did not match the exact contract"
    );
}

fn component_bytes(path: &RelativePath) -> Vec<Vec<u8>> {
    path.components().map(<[u8]>::to_vec).collect()
}

#[test]
fn identity_accessors_and_display_are_exact() {
    let filesystem_cases = [
        (
            "filesystem-linux",
            FilesystemIdentity {
                device: 17,
                kind: FilesystemKind::LinuxExtFamily,
                mount_id: 29,
            },
            17,
            FilesystemKind::LinuxExtFamily,
            29,
        ),
        (
            "filesystem-macos",
            FilesystemIdentity {
                device: 31,
                kind: FilesystemKind::MacOsApfs,
                mount_id: 43,
            },
            31,
            FilesystemKind::MacOsApfs,
            43,
        ),
    ];
    for (case_id, identity, expected_device, expected_kind, expected_mount_id) in filesystem_cases {
        assert_eq!(
            identity.device(),
            expected_device,
            "{case_id}: device accessor"
        );
        assert_eq!(identity.kind(), expected_kind, "{case_id}: kind accessor");
        assert_eq!(
            identity.mount_id(),
            expected_mount_id,
            "{case_id}: mount_id accessor"
        );
    }

    let object_cases = [
        (
            "object-first",
            ObjectIdentity {
                device: 17,
                inode: 23,
            },
            17,
            23,
        ),
        (
            "object-second",
            ObjectIdentity {
                device: 31,
                inode: 37,
            },
            31,
            37,
        ),
    ];
    for (case_id, identity, expected_device, expected_inode) in object_cases {
        assert_eq!(
            identity.device(),
            expected_device,
            "{case_id}: device accessor"
        );
        assert_eq!(
            identity.inode(),
            expected_inode,
            "{case_id}: inode accessor"
        );
    }

    let filesystem = FilesystemIdentity {
        device: 17,
        kind: FilesystemKind::LinuxExtFamily,
        mount_id: 29,
    };
    let object = ObjectIdentity {
        device: 17,
        inode: 23,
    };
    let authority = AuthorityIdentity { filesystem, object };
    assert_eq!(
        authority.filesystem(),
        filesystem,
        "authority-filesystem: filesystem accessor"
    );
    assert_eq!(
        authority.object(),
        object,
        "authority-object: object accessor"
    );
    assert_display(
        "authority-display-linux",
        authority,
        "device=17 inode=23 filesystem=LinuxExtFamily mount=29",
    );

    let macos_authority = AuthorityIdentity {
        filesystem: FilesystemIdentity {
            device: 31,
            kind: FilesystemKind::MacOsApfs,
            mount_id: 43,
        },
        object: ObjectIdentity {
            device: 31,
            inode: 37,
        },
    };
    assert_display(
        "authority-display-macos",
        macos_authority,
        "device=31 inode=37 filesystem=MacOsApfs mount=43",
    );
}

#[test]
fn adapter_kind_display_values_are_exact() {
    let cases = [
        (
            "adapter-configuration",
            AuthorityAdapterKind::Configuration,
            "configuration",
        ),
        ("adapter-source", AuthorityAdapterKind::Source, "source"),
        ("adapter-record", AuthorityAdapterKind::Record, "run-record"),
        ("adapter-process", AuthorityAdapterKind::Process, "process"),
        ("adapter-agent", AuthorityAdapterKind::Agent, "agent"),
        ("adapter-git", AuthorityAdapterKind::Git, "git"),
    ];

    for (case_id, kind, expected) in cases {
        assert_display(case_id, kind, expected);
    }
}

#[test]
fn path_error_display_values_are_exact() {
    let expected_identity = AuthorityIdentity {
        filesystem: FilesystemIdentity {
            device: 17,
            kind: FilesystemKind::LinuxExtFamily,
            mount_id: 29,
        },
        object: ObjectIdentity {
            device: 17,
            inode: 23,
        },
    };
    let actual_identity = AuthorityIdentity {
        filesystem: FilesystemIdentity {
            device: 31,
            kind: FilesystemKind::MacOsApfs,
            mount_id: 43,
        },
        object: ObjectIdentity {
            device: 31,
            inode: 37,
        },
    };

    let cases = [
        (
            "error-unsupported-platform",
            PathError::UnsupportedPlatform,
            "unsupported platform: only Linux and macOS are supported",
        ),
        (
            "error-unsupported-filesystem",
            PathError::UnsupportedFilesystem {
                path: "/var/lib/omnirepo".to_owned(),
                kind: "network".to_owned(),
            },
            "unsupported filesystem at \"/var/lib/omnirepo\": network; only local ext-family or APFS is supported",
        ),
        (
            "error-invalid-authority-root",
            PathError::InvalidAuthorityRoot {
                path: "/tmp/root".to_owned(),
                reason: "must be a directory".to_owned(),
            },
            "invalid authority root \"/tmp/root\": must be a directory",
        ),
        (
            "error-invalid-absolute-path",
            PathError::InvalidAbsolutePath {
                path: "/tmp/../escape".to_owned(),
                reason: "parent-directory traversal is not allowed".to_owned(),
            },
            "invalid absolute path \"/tmp/../escape\": parent-directory traversal is not allowed",
        ),
        (
            "error-invalid-relative-path",
            PathError::InvalidRelativePath {
                input: "../escape".to_owned(),
                reason: "parent-directory traversal is not allowed".to_owned(),
            },
            "invalid relative path \"../escape\": parent-directory traversal is not allowed",
        ),
        (
            "error-not-found",
            PathError::NotFound {
                path: "missing".to_owned(),
            },
            "authority path not found: \"missing\"",
        ),
        (
            "error-link-like-object",
            PathError::LinkLikeObject {
                path: "link".to_owned(),
            },
            "link-like object rejected without following: \"link\"",
        ),
        (
            "error-mount-crossing",
            PathError::MountCrossing {
                path: "mount".to_owned(),
            },
            "filesystem boundary crossed below authority root: \"mount\"",
        ),
        (
            "error-unsupported-object-directory",
            PathError::UnsupportedObject {
                path: "payload".to_owned(),
                expected: ObjectClass::Directory,
            },
            "unsupported object at \"payload\"; expected Directory",
        ),
        (
            "error-unsupported-object-any",
            PathError::UnsupportedObject {
                path: "entry".to_owned(),
                expected: ObjectClass::Any,
            },
            "unsupported object at \"entry\"; expected Any",
        ),
        (
            "error-unsafe-hard-link",
            PathError::UnsafeHardLink {
                path: "alias".to_owned(),
                links: 2,
            },
            "mutation target \"alias\" has 2 hard-link names and is unsafe",
        ),
        (
            "error-concurrent-replacement",
            PathError::ConcurrentReplacement {
                path: "target".to_owned(),
                reason: "ancestor changed".to_owned(),
            },
            "authority path was replaced while a mutation was in flight: \"target\": ancestor changed",
        ),
        (
            "error-authority-mismatch",
            PathError::AuthorityMismatch {
                owner: AuthorityAdapterKind::Source,
                root: "/source".to_owned(),
                expected: expected_identity,
                actual: actual_identity,
            },
            "source adapter target is outside owning root \"/source\": expected device=17 inode=23 filesystem=LinuxExtFamily mount=29, got device=31 inode=37 filesystem=MacOsApfs mount=43",
        ),
        (
            "error-duplicate-authority",
            PathError::DuplicateAuthority {
                label: "source".to_owned(),
                existing: "existing".to_owned(),
                identity: expected_identity,
            },
            "authority \"source\" duplicates \"existing\" (device=17 inode=23 filesystem=LinuxExtFamily mount=29)",
        ),
        (
            "error-authority-overlap",
            PathError::AuthorityOverlap {
                path: "nested".to_owned(),
            },
            "undeclared authority overlap at \"nested\"",
        ),
        (
            "error-io-some-code",
            PathError::Io {
                operation: "open".to_owned(),
                path: "target".to_owned(),
                kind: "permission denied".to_owned(),
                code: Some(13),
            },
            "open failed for authority path \"target\": permission denied (errno=Some(13))",
        ),
        (
            "error-io-no-code",
            PathError::Io {
                operation: "read".to_owned(),
                path: "target".to_owned(),
                kind: "unexpected end of file".to_owned(),
                code: None,
            },
            "read failed for authority path \"target\": unexpected end of file (errno=None)",
        ),
    ];

    for (case_id, error, expected) in cases {
        assert_display(case_id, error, expected);
    }
}

#[test]
fn relative_path_root_components_normalization_and_unicode_are_exact() {
    let root = RelativePath::root();
    assert_eq!(
        component_bytes(&root),
        Vec::<Vec<u8>>::new(),
        "relative-root-components"
    );
    assert_display("relative-root-display", root.display(), "");

    let normalized = RelativePath::parse("one//./two/").expect("relative-normalized: valid path");
    assert_eq!(
        component_bytes(&normalized),
        vec![b"one".to_vec(), b"two".to_vec()],
        "relative-normalized-components"
    );
    assert_display(
        "relative-normalized-display",
        normalized.display(),
        "one/two",
    );

    let direct = RelativePath::parse("one/two").expect("relative-direct: valid path");
    assert_eq!(normalized, direct, "relative-normalized-equivalence");

    let unicode = RelativePath::parse("café/数据").expect("relative-unicode: valid path");
    assert_eq!(
        component_bytes(&unicode),
        vec!["café".as_bytes().to_vec(), "数据".as_bytes().to_vec()],
        "relative-unicode-components"
    );
    assert_display("relative-unicode-display", unicode.display(), "café/数据");
}

#[test]
fn relative_path_invalid_cases_preserve_exact_reason_and_display() {
    let cases = [
        (
            "relative-empty",
            "",
            "an empty path is reserved for RelativePath::root()",
            "invalid relative path \"\": an empty path is reserved for RelativePath::root()",
        ),
        (
            "relative-nul",
            "nul\0component",
            "NUL is not a path component",
            "invalid relative path \"nul\\0component\": NUL is not a path component",
        ),
        (
            "relative-absolute",
            "/absolute",
            "absolute paths are not valid nested references",
            "invalid relative path \"/absolute\": absolute paths are not valid nested references",
        ),
        (
            "relative-parent",
            "one/../escape",
            "parent-directory traversal is not allowed",
            "invalid relative path \"one/../escape\": parent-directory traversal is not allowed",
        ),
        (
            "relative-no-component",
            ".",
            "the path has no component; use RelativePath::root() explicitly",
            "invalid relative path \".\": the path has no component; use RelativePath::root() explicitly",
        ),
    ];

    for (case_id, input, reason, expected_display) in cases {
        let actual = RelativePath::parse(input).expect_err(case_id);
        let expected = PathError::InvalidRelativePath {
            input: input.to_owned(),
            reason: reason.to_owned(),
        };
        assert_eq!(actual, expected, "{case_id}: invalid path value");
        assert_display(case_id, actual, expected_display);
    }
}

#[test]
fn absolute_path_valid_parse_from_path_and_access_are_exact() {
    for (case_id, input) in [
        ("absolute-basic", "/authority/root"),
        ("absolute-unicode", "/authority/café"),
    ] {
        let parsed = AbsolutePath::parse(input).expect(case_id);
        assert_eq!(
            parsed.as_path(),
            Path::new(input),
            "{case_id}: parse/as_path must preserve exact path"
        );
    }

    let with_curdir = PathBuf::from("/authority/./root");
    let from_path = AbsolutePath::from_path(&with_curdir).expect("absolute-from-path: valid path");
    assert_eq!(
        from_path.as_path(),
        with_curdir.as_path(),
        "absolute-from-path: as_path must preserve the supplied path"
    );

    let filesystem_root = AbsolutePath::from_path(Path::new("/"))
        .expect("absolute-filesystem-root: root path is a valid absolute path");
    assert_eq!(
        filesystem_root.as_path(),
        Path::new("/"),
        "absolute-filesystem-root: as_path"
    );
}

#[test]
fn absolute_path_invalid_cases_preserve_exact_reason_and_display() {
    let cases = [
        (
            "absolute-relative",
            "relative/root",
            "an authority root must be absolute",
            "invalid absolute path \"relative/root\": an authority root must be absolute",
        ),
        (
            "absolute-parent",
            "/authority/../escape",
            "parent-directory traversal is not allowed",
            "invalid absolute path \"/authority/../escape\": parent-directory traversal is not allowed",
        ),
        (
            "absolute-nul",
            "/nul\0root",
            "NUL is not a path component",
            "invalid absolute path \"/nul\\0root\": NUL is not a path component",
        ),
    ];

    for (case_id, input, reason, expected_display) in cases {
        let actual = AbsolutePath::parse(input).expect_err(case_id);
        let expected = PathError::InvalidAbsolutePath {
            path: input.to_owned(),
            reason: reason.to_owned(),
        };
        assert_eq!(actual, expected, "{case_id}: invalid path value");
        assert_display(case_id, actual, expected_display);
    }

    // On Unix, `to_str()` rejects the complete path before the per-component
    // `Component::Normal` UTF-8 check can run. This is the supported way to
    // cover the non-UTF-8 absolute-path boundary without fabricating private
    // invalid path state.
    use std::os::unix::ffi::OsStringExt;
    let non_utf8 = PathBuf::from(OsString::from_vec(b"/authority-\xff".to_vec()));
    let actual = AbsolutePath::from_path(&non_utf8).expect_err("absolute-non-utf8");
    let expected = PathError::InvalidAbsolutePath {
        path: "/authority-�".to_owned(),
        reason: "authority paths must be UTF-8".to_owned(),
    };
    assert_eq!(actual, expected, "absolute-non-utf8: invalid path value");
    assert_display(
        "absolute-non-utf8",
        actual,
        "invalid absolute path \"/authority-�\": authority paths must be UTF-8",
    );

    // `Component::Prefix` exists only on Windows. This Unix-only coverage
    // module documents that platform-specific branch instead of faking it.
}
