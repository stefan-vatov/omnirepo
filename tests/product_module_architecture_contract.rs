//! Fixture-based contract for the private product module topology.
//!
//! The validator deliberately uses maintained Rust, TOML, and Cargo parsers.
//! It is a test-only contract: the production binary does not depend on any of
//! these crates. Fixture validation stays green while the tracer-bullet move is
//! in progress; the live-root seam is enabled by the final move bead.

use cargo_metadata::{MetadataCommand, TargetKind};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::Mutex,
};
use syn::{
    Attribute, ExprMacro, File, Ident, Item, ItemMod, Meta, StmtMacro, UseTree, Visibility,
    ext::IdentExt,
    visit::{self, Visit},
};
use toml_edit::{DocumentMut, Item as TomlItem, Value};

const CONTEXTS: &[&str] = &[
    "configuration",
    "lifecycle",
    "managed_content",
    "platform",
    "repository",
    "source",
];
const MAX_SOURCE_FILES: usize = 10_000;
const MAX_SOURCE_DEPTH: usize = 64;
const CATCH_ALL_MODULES: &[&str] = &["common", "util", "utils", "prelude"];
const PRIVATE_TOOLS: &[(&str, &str)] = &[
    ("tools/omnirepo-dev", "omnirepo-dev"),
    ("tools/omnirepo-test-support", "omnirepo-test-support"),
];
const ALLOWED_STATEMENT_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "cfg",
    "dbg",
    "eprint",
    "eprintln",
    "format_args",
    "panic",
    "print",
    "println",
    "todo",
    "unimplemented",
    "unreachable",
    "vec",
    "write",
    "writeln",
];
static CARGO_GATE: Mutex<()> = Mutex::new(());

/// Validate a product root without changing it.
pub fn validate_live_root(root: &Path) -> Result<(), Vec<String>> {
    validate_root(root)
}

#[derive(Default)]
struct ModuleGraph {
    reachable: BTreeSet<PathBuf>,
    test_only: BTreeSet<PathBuf>,
    visiting: BTreeSet<PathBuf>,
    edges: BTreeSet<(String, String)>,
    edge_occurrences: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModuleMode {
    Runtime,
    TestOnly,
}

#[derive(Clone, Debug)]
struct AliasSpec {
    alias: Option<String>,
    raw_path: Vec<String>,
}

#[derive(Default)]
struct ManifestInfo {
    document: Option<DocumentMut>,
    workspace_members: Vec<String>,
    workspace_dependency_names: BTreeSet<String>,
    dependency_paths: Vec<String>,
    workspace_dependency_paths: BTreeMap<String, String>,
    workspace_inherited_dependencies: BTreeSet<String>,
}

fn validate_root(input_root: &Path) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    let root = match lexical_absolute(input_root) {
        Ok(root) => root,
        Err(error) => {
            return Err(vec![format!("repository root path is invalid: {error}")]);
        }
    };

    if !require_directory(&root, "repository root", &mut errors) {
        return Err(errors);
    }
    if !require_regular_file(&root.join("Cargo.toml"), "Cargo.toml", &mut errors)
        || !require_directory(&root.join("src"), "src", &mut errors)
        || !require_regular_file(&root.join("src/main.rs"), "src/main.rs", &mut errors)
    {
        return Err(errors);
    }

    let manifest_text = match read_utf8(&root.join("Cargo.toml"), "Cargo.toml", &mut errors) {
        Some(text) => text,
        None => return Err(errors),
    };
    let manifest = match parse_manifest(&manifest_text, "Cargo.toml", &mut errors) {
        Some(manifest) => manifest,
        None => return Err(errors),
    };
    validate_manifest_shapes(&manifest, &mut errors);
    validate_manifest_targets(&manifest, &mut errors);

    let main_text = match read_utf8(&root.join("src/main.rs"), "src/main.rs", &mut errors) {
        Some(text) => text,
        None => return Err(errors),
    };
    let main_file = match parse_rust(&main_text, &root.join("src/main.rs"), &mut errors) {
        Some(file) => file,
        None => return Err(errors),
    };

    validate_workspace_files(&root, &manifest, &mut errors);
    validate_product_path_dependencies(&root, &manifest, &mut errors);

    let mut graph = ModuleGraph::default();
    validate_main_shape(&main_file, &root.join("src"), &mut graph, &mut errors);
    audit_file(&main_file, &root.join("src/main.rs"), &mut errors);
    graph.reachable.insert(PathBuf::from("src/main.rs"));
    walk_modules_from_items(
        &main_file.items,
        &root.join("src/main.rs"),
        &root.join("src"),
        &mut graph,
        &mut errors,
        ModuleMode::Runtime,
    );

    let mut source_files = Vec::new();
    validate_source_root_entries(&root.join("src"), &mut errors);
    collect_rust_sources(
        &root.join("src"),
        &root.join("src"),
        0,
        &mut source_files,
        &mut errors,
    );
    for path in &source_files {
        let relative = path
            .strip_prefix(&root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf());
        if !graph.reachable.contains(&relative) && !graph.test_only.contains(&relative) {
            errors.push(format!(
                "runtime Rust source is not reachable from the private module graph: {}",
                relative.display()
            ));
        }
    }

    validate_edges(&graph.edges, &graph.edge_occurrences, &mut errors);
    validate_cargo_authority(&root, &manifest, &source_files, &graph, &mut errors);

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_manifest_shapes(manifest: &ManifestInfo, errors: &mut Vec<String>) {
    let Some(document) = manifest.document.as_ref() else {
        return;
    };
    if let Some(package) = document.get("package").and_then(TomlItem::as_table) {
        for key in ["name", "autobins"] {
            if let Some(item) = package.get(key) {
                let valid = if key == "name" {
                    item.as_str().is_some()
                } else {
                    item.as_bool().is_some()
                };
                if !valid {
                    errors.push(format!("unsupported [package] value shape for {key}"));
                }
            }
        }
        for key in ["include", "exclude"] {
            if let Some(item) = package.get(key) {
                let valid = item
                    .as_array()
                    .is_some_and(|array| array.iter().all(|value| value.as_str().is_some()));
                if !valid {
                    errors.push(format!("[package] {key} must be a string array"));
                }
            }
        }
    }
    for (key, item) in document.iter() {
        if key != "bin" && item.as_array_of_tables().is_some() {
            errors.push(format!("unsupported Cargo target table [[{key}]]"));
        }
    }
}

fn validate_manifest_targets(manifest: &ManifestInfo, errors: &mut Vec<String>) {
    let Some(document) = manifest.document.as_ref() else {
        return;
    };
    let autobins_false = document
        .get("package")
        .and_then(TomlItem::as_table)
        .and_then(|package| package.get("autobins"))
        .and_then(TomlItem::as_bool)
        == Some(false);
    let bins = document.get("bin").and_then(TomlItem::as_array_of_tables);
    let bin_count = bins.map_or(0, |bins| bins.iter().count());
    if bin_count > 1 {
        errors.push("duplicate binary target declarations are not allowed".into());
        errors.push("product must not declare multiple binary targets".into());
    }
    if autobins_false && bin_count == 0 {
        errors.push("autobins=false requires one explicit binary target".into());
    }
    if let Some(bin) = bins.and_then(|bins| bins.iter().next()) {
        let name = bin.get("name").and_then(TomlItem::as_str);
        let path = bin.get("path").and_then(TomlItem::as_str);
        if name != Some("omnirepo") {
            errors.push("binary name must be omnirepo".into());
        }
        if path != Some("src/main.rs") {
            errors.push("binary path must be src/main.rs".into());
        }
    }
}

fn validate_source_root_entries(src: &Path, errors: &mut Vec<String>) {
    let entries = match fs::read_dir(src) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!(
                "cannot enumerate product source root {}: {error}",
                src.display()
            ));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!("cannot inspect product source entry: {error}"));
                continue;
            }
        };
        let path = entry.path();
        let Some(metadata) = no_follow_metadata(&path, errors) else {
            continue;
        };
        if metadata.is_file()
            && path.extension().is_some_and(|extension| extension == "rs")
            && path.file_name().is_some_and(|name| name != "main.rs")
        {
            errors.push(format!(
                "unowned top-level product Rust file {}",
                path.display()
            ));
        }
        if metadata.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if !CONTEXTS.contains(&name) {
                errors.push(format!("unexpected or catch-all product module {name:?}"));
            }
        }
    }
}

fn lexical_absolute(path: &Path) -> Result<PathBuf, String> {
    let base = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| format!("cannot determine current directory: {error}"))?
            .join(path)
    };
    lexical_normalize(&base)
}

fn lexical_normalize(path: &Path) -> Result<PathBuf, String> {
    let mut result = if path.is_absolute() {
        PathBuf::new()
    } else {
        return Err(format!("path is not absolute: {}", path.display()));
    };
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => result.push(prefix.as_os_str()),
            Component::RootDir => result.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !result.pop() {
                    return Err(format!("path escapes its root: {}", path.display()));
                }
            }
            Component::Normal(component) => result.push(component),
        }
    }
    Ok(result)
}

fn require_directory(path: &Path, label: &str, errors: &mut Vec<String>) -> bool {
    match no_follow_metadata(path, errors) {
        Some(metadata) if metadata.is_dir() => true,
        Some(_) => {
            errors.push(format!("{label} must be a directory: {}", path.display()));
            false
        }
        None => false,
    }
}

fn require_regular_file(path: &Path, label: &str, errors: &mut Vec<String>) -> bool {
    match no_follow_metadata(path, errors) {
        Some(metadata) if metadata.file_type().is_file() => true,
        Some(_) => {
            errors.push(format!(
                "{label} must be a regular file: {}",
                path.display()
            ));
            false
        }
        None => false,
    }
}

fn no_follow_metadata(path: &Path, errors: &mut Vec<String>) -> Option<fs::Metadata> {
    inspect_no_follow(path, errors, false).flatten()
}

fn no_follow_exists(path: &Path, errors: &mut Vec<String>) -> Option<bool> {
    inspect_no_follow(path, errors, true).map(|metadata| metadata.is_some())
}

fn inspect_no_follow(
    path: &Path,
    errors: &mut Vec<String>,
    allow_missing: bool,
) -> Option<Option<fs::Metadata>> {
    let mut current = PathBuf::new();
    let mut final_metadata = None;
    for component in path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                errors.push(format!("path must not be a symlink: {}", current.display()));
                return None;
            }
            Ok(metadata) => final_metadata = Some(metadata),
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
                return Some(None);
            }
            Err(error) => {
                errors.push(format!(
                    "cannot inspect path {}: {error}",
                    current.display()
                ));
                return None;
            }
        }
    }
    Some(final_metadata)
}

fn read_utf8(path: &Path, label: &str, errors: &mut Vec<String>) -> Option<String> {
    if !require_regular_file(path, label, errors) {
        return None;
    }
    match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) => {
            errors.push(format!("cannot read {label} {}: {error}", path.display()));
            None
        }
    }
}

fn parse_rust(source: &str, path: &Path, errors: &mut Vec<String>) -> Option<File> {
    match syn::parse_file(source) {
        Ok(file) => Some(file),
        Err(error) => {
            errors.push(format!("Rust parser rejected {}: {error}", path.display()));
            None
        }
    }
}

fn parse_manifest(source: &str, path: &str, errors: &mut Vec<String>) -> Option<ManifestInfo> {
    // Cargo supports multiline strings, but this contract deliberately rejects
    // unsupported relevant shapes instead of treating them as an opaque guess.
    if source.contains("\"\"\"") || source.contains("'''") {
        errors.push(format!("unsupported multiline TOML shape in {path}"));
    }
    let document = match source.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(error) => {
            errors.push(format!("TOML parser rejected {path}: {error}"));
            return None;
        }
    };
    let mut manifest = ManifestInfo {
        document: Some(document.clone()),
        ..ManifestInfo::default()
    };
    if let Some(workspace) = document.get("workspace").and_then(TomlItem::as_table) {
        if let Some(members) = workspace.get("members").and_then(TomlItem::as_array) {
            for value in members.iter() {
                if let Some(member) = value.as_str() {
                    manifest.workspace_members.push(member.to_owned());
                } else {
                    errors.push("workspace.members must contain only strings".into());
                }
            }
        } else {
            errors.push("workspace.members must be an array of strings".into());
        }
    }

    let mut prefix = Vec::new();
    visit_toml_table(document.as_table(), &mut prefix, &mut manifest, errors);
    Some(manifest)
}

fn visit_toml_table(
    table: &toml_edit::Table,
    prefix: &mut Vec<String>,
    manifest: &mut ManifestInfo,
    errors: &mut Vec<String>,
) {
    for (key, item) in table.iter() {
        prefix.push(key.to_owned());
        visit_toml_item(item, prefix, manifest, errors);
        prefix.pop();
    }
}

fn visit_toml_item(
    item: &TomlItem,
    prefix: &[String],
    manifest: &mut ManifestInfo,
    errors: &mut Vec<String>,
) {
    if let Some(table) = item.as_table() {
        let mut nested = prefix.to_vec();
        visit_toml_table(table, &mut nested, manifest, errors);
        return;
    }
    if let Some(inline) = item.as_inline_table() {
        for (key, value) in inline.iter() {
            let mut nested = prefix.to_vec();
            nested.push(key.to_owned());
            visit_toml_value(value, &nested, manifest, errors);
        }
        return;
    }
    if let Some(array_of_tables) = item.as_array_of_tables() {
        for table in array_of_tables.iter() {
            let mut nested = prefix.to_vec();
            visit_toml_table(table, &mut nested, manifest, errors);
        }
        return;
    }
    if let Some(value) = item.as_value() {
        visit_toml_value(value, prefix, manifest, errors);
    }
}

fn visit_toml_value(
    value: &Value,
    prefix: &[String],
    manifest: &mut ManifestInfo,
    errors: &mut Vec<String>,
) {
    let dependency_index = prefix.iter().position(|part| {
        matches!(
            part.as_str(),
            "dependencies" | "dev-dependencies" | "build-dependencies"
        )
    });
    let Some(index) = dependency_index else {
        return;
    };
    if index + 1 >= prefix.len() {
        errors.push(format!(
            "unsupported dependency table shape: {}",
            prefix.join(".")
        ));
        return;
    }
    let dependency_name = &prefix[index + 1];
    let field = prefix.get(index + 2).map(String::as_str);
    let is_workspace_table = prefix
        .get(index.wrapping_sub(1))
        .is_some_and(|part| part == "workspace")
        && index > 0;
    match field {
        None => {
            if is_workspace_table {
                manifest
                    .workspace_dependency_names
                    .insert(dependency_name.to_owned());
            }
            if !value.is_str() && !value.is_inline_table() && !value.is_array() {
                errors.push(format!(
                    "unsupported dependency declaration shape for {dependency_name}"
                ));
            }
        }
        Some("path") => {
            let Some(path) = value.as_str() else {
                errors.push(format!(
                    "dependency path must be a string for {dependency_name}"
                ));
                return;
            };
            if is_workspace_table {
                manifest
                    .workspace_dependency_names
                    .insert(dependency_name.to_owned());
                manifest
                    .workspace_dependency_paths
                    .insert(dependency_name.to_owned(), path.to_owned());
            } else {
                manifest.dependency_paths.push(path.to_owned());
            }
        }
        Some("workspace") => {
            if !value.is_bool() {
                errors.push(format!(
                    "dependency workspace flag must be boolean for {dependency_name}"
                ));
            } else if value.as_bool() == Some(true) && !is_workspace_table {
                manifest
                    .workspace_inherited_dependencies
                    .insert(dependency_name.to_owned());
            }
        }
        Some(_) if prefix.len() > index + 3 => {
            errors.push(format!(
                "unsupported nested dependency shape: {}",
                prefix.join(".")
            ));
        }
        Some(_) => {
            // Cargo dependency fields are intentionally not reinterpreted here.
            // Their syntax is owned by Cargo; path and workspace fields above
            // are the only fields needed for this boundary.
        }
    }
}

fn validate_workspace_files(root: &Path, manifest: &ManifestInfo, errors: &mut Vec<String>) {
    let expected = BTreeMap::from([
        ("tools/omnirepo-dev", "omnirepo-dev"),
        ("tools/omnirepo-test-support", "omnirepo-test-support"),
    ]);
    let actual: BTreeSet<_> = manifest
        .workspace_members
        .iter()
        .map(|member| normalize_manifest_path(member))
        .collect();
    let expected_paths: BTreeSet<_> = expected.keys().map(|path| (*path).to_owned()).collect();
    if actual != expected_paths || manifest.workspace_members.len() != expected.len() {
        errors.push(format!(
            "workspace members must be exactly the two private tools, got {:?}",
            manifest.workspace_members
        ));
    }

    for (relative, package_name) in expected {
        let member_root = root.join(relative);
        let member_manifest = member_root.join("Cargo.toml");
        if !require_directory(&member_root, "workspace member", errors)
            || !require_regular_file(&member_manifest, "workspace member manifest", errors)
        {
            continue;
        }
        let Some(text) = read_utf8(&member_manifest, relative, errors) else {
            continue;
        };
        let Some(info) = parse_manifest(&text, relative, errors) else {
            continue;
        };
        let Some(document) = info.document else {
            continue;
        };
        let package = document.get("package").and_then(TomlItem::as_table);
        let actual_name = package
            .and_then(|table| table.get("name"))
            .and_then(TomlItem::as_str);
        if actual_name != Some(package_name) {
            errors.push(format!(
                "workspace member package name must be {package_name}: {relative}"
            ));
        }
        let publish_false = package
            .and_then(|table| table.get("publish"))
            .and_then(TomlItem::as_bool)
            == Some(false);
        if !publish_false {
            errors.push(format!(
                "workspace member must set package publish = false: {relative}"
            ));
        }
    }
}

fn normalize_manifest_path(path: &str) -> String {
    path.replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .fold(Vec::<String>::new(), |mut parts, part| {
            if part == ".." {
                let _ = parts.pop();
            } else {
                parts.push(part.to_owned());
            }
            parts
        })
        .join("/")
}

fn validate_product_path_dependencies(
    root: &Path,
    manifest: &ManifestInfo,
    errors: &mut Vec<String>,
) {
    let root_id = match lexical_absolute(root) {
        Ok(path) => path,
        Err(error) => {
            errors.push(format!("cannot identify product root: {error}"));
            return;
        }
    };
    for path in manifest
        .dependency_paths
        .iter()
        .chain(manifest.workspace_dependency_paths.values())
    {
        let Some(candidate) = resolve_contained_path(&root_id, path, errors) else {
            continue;
        };
        if !path_exists_and_is_contained(&root_id, &candidate, errors) {
            continue;
        }
        for (tool_path, tool_name) in PRIVATE_TOOLS {
            let tool = root_id.join(tool_path);
            let Ok(tool) = lexical_normalize(&tool) else {
                continue;
            };
            if candidate == tool || candidate.starts_with(&tool) {
                errors.push(format!(
                    "product must not depend on workspace tool crate: {path}"
                ));
                let _ = tool_name;
            }
        }
    }
    for dependency in &manifest.workspace_inherited_dependencies {
        if !manifest.workspace_dependency_names.contains(dependency) {
            errors.push(format!(
                "workspace-inherited dependency {dependency} is not declared in workspace.dependencies"
            ));
        }
    }
}

fn resolve_contained_path(root: &Path, raw: &str, errors: &mut Vec<String>) -> Option<PathBuf> {
    let raw_path = Path::new(raw);
    let joined = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        root.join(raw_path)
    };
    let candidate = match lexical_normalize(&joined) {
        Ok(candidate) => candidate,
        Err(error) => {
            errors.push(format!(
                "dependency path escapes the product root: {raw} ({error})"
            ));
            return None;
        }
    };
    if !candidate.starts_with(root) {
        errors.push(format!("dependency path escapes the product root: {raw}"));
        return None;
    }
    Some(candidate)
}

fn path_exists_and_is_contained(root: &Path, path: &Path, errors: &mut Vec<String>) -> bool {
    if !path.starts_with(root) {
        errors.push(format!("path escapes the product root: {}", path.display()));
        return false;
    }
    let relative = match path.strip_prefix(root) {
        Ok(relative) => relative,
        Err(_) => return false,
    };
    let mut current = root.to_path_buf();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            errors.push(format!("path escapes the product root: {}", path.display()));
            return false;
        }
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                errors.push(format!(
                    "dependency path must not contain a symlink: {}",
                    current.display()
                ));
                return false;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                errors.push(format!(
                    "dependency path does not exist: {}",
                    path.display()
                ));
                return false;
            }
            Err(error) => {
                errors.push(format!(
                    "cannot inspect dependency path {}: {error}",
                    current.display()
                ));
                return false;
            }
        }
    }
    true
}

fn validate_main_shape(file: &File, src: &Path, graph: &mut ModuleGraph, errors: &mut Vec<String>) {
    if !file.attrs.is_empty() {
        errors.push("src/main.rs must not have outer attributes".into());
    }
    if file.items.len() != CONTEXTS.len() + 1 {
        errors.push(
            "src/main.rs must contain exactly six private modules followed by fn main".into(),
        );
    }
    for (index, context) in CONTEXTS.iter().enumerate() {
        let Some(Item::Mod(module)) = file.items.get(index) else {
            errors.push(format!(
                "src/main.rs is missing private module declaration mod {context};"
            ));
            continue;
        };
        if ident_name(&module.ident) != *context
            || !matches!(module.vis, Visibility::Inherited)
            || module.content.is_some()
            || module.semi.is_none()
            || module.unsafety.is_some()
            || !module.attrs.is_empty()
        {
            errors.push(format!(
                "src/main.rs must contain only the exact private file-backed declaration mod {context};"
            ));
        }
    }
    let Some(Item::Fn(function)) = file.items.get(CONTEXTS.len()) else {
        errors.push("src/main.rs must end with one composition fn main()".into());
        return;
    };
    let signature = &function.sig;
    let exact_signature = ident_name(&signature.ident) == "main"
        && matches!(function.vis, Visibility::Inherited)
        && function.attrs.is_empty()
        && signature.constness.is_none()
        && signature.asyncness.is_none()
        && matches!(signature.safety, syn::Safety::Default)
        && signature.abi.is_none()
        && signature.generics.params.is_empty()
        && signature.inputs.is_empty()
        && signature.variadic.is_none()
        && matches!(signature.output, syn::ReturnType::Default)
        && function.modifiers.defaultness.is_none();
    if !exact_signature {
        errors
            .push("src/main.rs must contain the exact private fn main() composition entry".into());
    }
    let _ = (src, graph);
}

fn ident_name(ident: &Ident) -> String {
    ident.unraw().to_string()
}

fn audit_file(file: &File, path: &Path, errors: &mut Vec<String>) {
    audit_items_for_verbatim(&file.items, path, errors);
    let mut visitor = MacroAudit { path, errors };
    visitor.visit_file(file);
}

fn audit_items_for_verbatim(items: &[Item], path: &Path, errors: &mut Vec<String>) {
    for item in items {
        match item {
            Item::Verbatim(_) => errors.push(format!(
                "Rust parser produced an unsupported verbatim item in {}",
                path.display()
            )),
            Item::Mod(module) => {
                if let Some((_, nested)) = &module.content {
                    audit_items_for_verbatim(nested, path, errors);
                }
            }
            Item::Impl(item) => {
                for nested in &item.items {
                    if matches!(nested, syn::ImplItem::Verbatim(_)) {
                        errors.push(format!(
                            "Rust parser produced an unsupported verbatim impl item in {}",
                            path.display()
                        ));
                    }
                }
            }
            Item::Trait(item) => {
                for nested in &item.items {
                    if matches!(nested, syn::TraitItem::Verbatim(_)) {
                        errors.push(format!(
                            "Rust parser produced an unsupported verbatim trait item in {}",
                            path.display()
                        ));
                    }
                }
            }
            Item::ForeignMod(item) => {
                for nested in &item.items {
                    if matches!(nested, syn::ForeignItem::Verbatim(_)) {
                        errors.push(format!(
                            "Rust parser produced an unsupported verbatim foreign item in {}",
                            path.display()
                        ));
                    }
                }
            }
            _ => {}
        }
    }
}

struct MacroAudit<'a> {
    path: &'a Path,
    errors: &'a mut Vec<String>,
}

impl<'ast> Visit<'ast> for MacroAudit<'_> {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if attribute.path().is_ident("cfg_attr") {
            self.errors.push(format!(
                "cfg_attr source or configuration escape is unsupported: {}",
                self.path.display()
            ));
        }
        if attribute.path().is_ident("path") {
            self.errors.push(format!(
                "#[path] source overrides are unsupported: {}",
                self.path.display()
            ));
        }
        visit::visit_attribute(self, attribute);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        let macro_name = item
            .mac
            .path
            .segments
            .last()
            .map(|segment| ident_name(&segment.ident))
            .unwrap_or_default();
        if matches!(
            macro_name.as_str(),
            "macro_rules" | "slug_type" | "opaque_id" | "text_value" | "thread_local"
        ) {
            visit::visit_item_macro(self, item);
            return;
        }
        self.errors.push(format!(
            "item-position graph macro is unsupported: {}",
            self.path.display()
        ));
        let name = item
            .mac
            .path
            .segments
            .last()
            .map(|segment| ident_name(&segment.ident))
            .unwrap_or_default();
        if matches!(name.as_str(), "include" | "include_str" | "include_bytes") {
            self.errors.push(format!(
                "include source macro is unsupported: {}",
                self.path.display()
            ));
        }
        visit::visit_item_macro(self, item);
    }

    fn visit_impl_item_macro(&mut self, item: &'ast syn::ImplItemMacro) {
        self.errors.push(format!(
            "impl-position graph macro is unsupported: {}",
            self.path.display()
        ));
        visit::visit_impl_item_macro(self, item);
    }

    fn visit_trait_item_macro(&mut self, item: &'ast syn::TraitItemMacro) {
        self.errors.push(format!(
            "trait-position graph macro is unsupported: {}",
            self.path.display()
        ));
        visit::visit_trait_item_macro(self, item);
    }

    fn visit_foreign_item_macro(&mut self, item: &'ast syn::ForeignItemMacro) {
        self.errors.push(format!(
            "foreign-position graph macro is unsupported: {}",
            self.path.display()
        ));
        visit::visit_foreign_item_macro(self, item);
    }

    fn visit_expr_macro(&mut self, item: &'ast ExprMacro) {
        let name = item
            .mac
            .path
            .segments
            .last()
            .map(|segment| ident_name(&segment.ident))
            .unwrap_or_default();
        if matches!(name.as_str(), "include" | "include_str" | "include_bytes") {
            self.errors.push(format!(
                "include source macro is unsupported: {}",
                self.path.display()
            ));
        }
        visit::visit_expr_macro(self, item);
    }

    fn visit_stmt_macro(&mut self, item: &'ast StmtMacro) {
        let name = item
            .mac
            .path
            .segments
            .last()
            .map(|segment| ident_name(&segment.ident))
            .unwrap_or_default();
        if !ALLOWED_STATEMENT_MACROS.contains(&name.as_str()) {
            self.errors.push(format!(
                "ambiguous statement macro is unsupported: {}",
                self.path.display()
            ));
        }
        if matches!(name.as_str(), "include" | "include_str" | "include_bytes") {
            self.errors.push(format!(
                "include source macro is unsupported: {}",
                self.path.display()
            ));
        }
        visit::visit_stmt_macro(self, item);
    }
}

fn module_mode(module: &ItemMod, errors: &mut Vec<String>) -> ModuleMode {
    let mut cfg_test = false;
    let mut other_cfg = false;
    for attribute in &module.attrs {
        if attribute.path().is_ident("cfg") {
            match &attribute.meta {
                Meta::List(list) if list.tokens.to_string().trim() == "test" => cfg_test = true,
                Meta::List(list) if supported_target_cfg(&list.tokens.to_string()) => {}
                Meta::List(_) => {
                    other_cfg = true;
                    errors.push(format!(
                        "unsupported cfg predicate on module {}",
                        ident_name(&module.ident)
                    ));
                }
                _ => {
                    other_cfg = true;
                    errors.push(format!(
                        "unsupported cfg predicate on module {}",
                        ident_name(&module.ident)
                    ));
                }
            }
        }
        if attribute.path().is_ident("cfg_attr") {
            other_cfg = true;
        }
    }
    if cfg_test && !other_cfg {
        ModuleMode::TestOnly
    } else {
        ModuleMode::Runtime
    }
}

fn supported_target_cfg(tokens: &str) -> bool {
    let normalized = tokens.replace(' ', "");
    normalized.contains("target_os") || normalized == "unix" || normalized.contains("not(test)")
}

fn walk_modules_from_items(
    items: &[Item],
    parent_file: &Path,
    src: &Path,
    graph: &mut ModuleGraph,
    errors: &mut Vec<String>,
    mode: ModuleMode,
) {
    for item in items {
        let Item::Mod(module) = item else {
            continue;
        };
        walk_module(module, parent_file, src, graph, errors, mode);
    }
}

fn walk_module(
    module: &ItemMod,
    parent_file: &Path,
    src: &Path,
    graph: &mut ModuleGraph,
    errors: &mut Vec<String>,
    parent_mode: ModuleMode,
) {
    let declared_mode = module_mode(module, errors);
    let mode = if parent_mode == ModuleMode::TestOnly || declared_mode == ModuleMode::TestOnly {
        ModuleMode::TestOnly
    } else {
        ModuleMode::Runtime
    };
    let name = ident_name(&module.ident);
    if CATCH_ALL_MODULES.contains(&name.as_str()) {
        errors.push(format!(
            "catch-all module name is not allowed in the private product module graph: {name}"
        ));
    }
    if mode == ModuleMode::Runtime && !matches!(module.vis, Visibility::Inherited) {
        errors.push(format!("runtime module {name} must remain private"));
    }

    if module.content.is_some() {
        let target_specific = module.attrs.iter().any(|attribute| {
            attribute.path().is_ident("cfg")
                && matches!(&attribute.meta, Meta::List(list) if supported_target_cfg(&list.tokens.to_string()))
        });
        if target_specific {
            return;
        }
        if mode == ModuleMode::Runtime {
            errors.push(format!("inline runtime module {name} is unsupported"));
        } else if let Some((_, nested)) = &module.content {
            walk_modules_from_items(nested, parent_file, src, graph, errors, mode);
        }
        return;
    }

    let Some(child_path) = module_source_path(parent_file, &name, src, errors) else {
        return;
    };
    let relative = child_path
        .strip_prefix(src.parent().unwrap_or(src))
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| child_path.clone());
    if graph.visiting.contains(&relative) {
        errors.push(format!(
            "private product module declaration graph contains a cycle at {}",
            relative.display()
        ));
        return;
    }
    let inserted = match mode {
        ModuleMode::Runtime => graph.reachable.insert(relative.clone()),
        ModuleMode::TestOnly => graph.test_only.insert(relative.clone()),
    };
    let already_other_mode = match mode {
        ModuleMode::Runtime => graph.test_only.contains(&relative),
        ModuleMode::TestOnly => graph.reachable.contains(&relative),
    };
    if already_other_mode {
        errors.push(format!(
            "source is both runtime and test-only: {}",
            relative.display()
        ));
    }
    if !inserted {
        return;
    }
    graph.visiting.insert(relative.clone());
    let Some(source) = read_utf8(&child_path, "module source", errors) else {
        graph.visiting.remove(&relative);
        return;
    };
    let Some(file) = parse_rust(&source, &child_path, errors) else {
        graph.visiting.remove(&relative);
        return;
    };
    audit_file(&file, &child_path, errors);
    let owner = module_owner(src, &child_path);
    if mode == ModuleMode::Runtime {
        let module_path = module_path_for_file(src, &child_path);
        let aliases = aliases_for_file(&file, &module_path, errors);
        let mut dependencies = BTreeSet::new();
        for dependency in use_dependencies(&file, &module_path, &aliases) {
            if dependency != owner {
                graph.edges.insert((owner.clone(), dependency.clone()));
                graph.edge_occurrences.push((owner.clone(), dependency));
            }
        }
        let mut visitor = DependencyVisitor {
            module_path: module_path_for_file(src, &child_path),
            aliases: &aliases,
            dependencies: &mut dependencies,
        };
        visitor.visit_file(&file);
        for dependency in dependencies {
            if dependency != owner {
                graph.edges.insert((owner.clone(), dependency.clone()));
                graph.edge_occurrences.push((owner.clone(), dependency));
            }
        }
    }
    walk_modules_from_items(&file.items, &child_path, src, graph, errors, mode);
    graph.visiting.remove(&relative);
}

fn module_source_path(
    parent_file: &Path,
    name: &str,
    src: &Path,
    errors: &mut Vec<String>,
) -> Option<PathBuf> {
    let parent = parent_file.parent().unwrap_or(src);
    let module_dir = if parent_file
        .file_name()
        .is_some_and(|file_name| file_name == "main.rs")
    {
        src.to_path_buf()
    } else if parent_file
        .file_name()
        .is_some_and(|file_name| file_name == "mod.rs")
    {
        parent.to_path_buf()
    } else {
        parent.join(parent_file.file_stem().unwrap_or_default())
    };
    let direct = module_dir.join(format!("{name}.rs"));
    let legacy_direct = parent.join(format!("{name}.rs"));
    let nested = module_dir.join(name).join("mod.rs");
    let direct_exists = no_follow_exists(&direct, errors)?;
    let legacy_direct_exists = if direct == legacy_direct {
        false
    } else {
        no_follow_exists(&legacy_direct, errors)?
    };
    let nested_exists = no_follow_exists(&nested, errors)?;
    if (direct_exists || legacy_direct_exists) && nested_exists {
        errors.push(format!(
            "module declaration has ambiguous source files: {} and {}",
            direct.display(),
            nested.display()
        ));
        return None;
    }
    if !direct_exists && !legacy_direct_exists && !nested_exists {
        errors.push(format!(
            "module declaration has no contained regular source file: {}",
            direct.display()
        ));
        return None;
    }
    let child = if direct_exists {
        direct
    } else if legacy_direct_exists {
        legacy_direct
    } else {
        nested
    };
    if !child.starts_with(src) {
        errors.push(format!("module source escapes src: {}", child.display()));
        return None;
    }
    if !require_regular_file(&child, "module source", errors) {
        return None;
    }
    Some(child)
}

fn module_owner(src: &Path, file: &Path) -> String {
    file.strip_prefix(src)
        .ok()
        .and_then(|relative| relative.components().next())
        .and_then(|component| component.as_os_str().to_str())
        .filter(|name| CONTEXTS.contains(name))
        .unwrap_or("main")
        .to_owned()
}

fn module_path_for_file(src: &Path, file: &Path) -> Vec<String> {
    let Ok(relative) = file.strip_prefix(src) else {
        return Vec::new();
    };
    let mut parts: Vec<String> = relative
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .map(str::to_owned)
        .collect();
    if let Some(last) = parts.last_mut() {
        if last == "mod.rs" {
            parts.pop();
        } else if let Some(stem) = last.strip_suffix(".rs") {
            *last = stem.to_owned();
        }
    }
    parts
}

struct DependencyVisitor<'a> {
    module_path: Vec<String>,
    aliases: &'a BTreeMap<String, Vec<String>>,
    dependencies: &'a mut BTreeSet<String>,
}

impl<'ast> Visit<'ast> for DependencyVisitor<'_> {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments: Vec<String> = path
            .segments
            .iter()
            .map(|segment| ident_name(&segment.ident))
            .collect();
        if let Some(resolved) = resolve_code_path(&segments, &self.module_path, self.aliases)
            && let Some(context) = resolved.iter().find(|part| {
                CONTEXTS.contains(&part.as_str())
                    && self.module_path.first().map(String::as_str) != Some(part.as_str())
            })
        {
            self.dependencies.insert(context.to_owned());
        }
        visit::visit_path(self, path);
    }
}

/// `syn` deliberately models a `use` declaration as a `UseTree`, not as a
/// `syn::Path`, so the normal path visitor never sees its dependency edge.
/// Resolve the structured use trees explicitly and merge their context edges
/// with expression/type paths visited by `DependencyVisitor`.
fn use_dependencies(
    file: &File,
    module_path: &[String],
    aliases: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    let mut specs = Vec::new();
    for item in &file.items {
        if let Item::Use(item_use) = item {
            let mut raw_prefix = Vec::new();
            collect_use_specs(&item_use.tree, &mut raw_prefix, &mut specs);
        }
    }
    specs
        .into_iter()
        .filter_map(|spec| resolve_raw_use_path(&spec.raw_path, module_path, aliases))
        .filter_map(|path| {
            path.into_iter().find(|part| {
                CONTEXTS.contains(&part.as_str())
                    && module_path.first().map(String::as_str) != Some(part.as_str())
            })
        })
        .collect()
}

fn aliases_for_file(
    file: &File,
    module_path: &[String],
    errors: &mut Vec<String>,
) -> BTreeMap<String, Vec<String>> {
    let mut specs = Vec::new();
    for item in &file.items {
        if let Item::Use(item_use) = item {
            let mut raw_prefix = Vec::new();
            collect_use_specs(&item_use.tree, &mut raw_prefix, &mut specs);
        }
    }
    let alias_names: BTreeSet<String> = specs
        .iter()
        .filter_map(|spec| spec.alias.as_deref())
        .map(str::to_owned)
        .collect();
    let mut aliases = BTreeMap::new();
    let mut external_aliases = BTreeSet::new();
    for _ in 0..=specs.len() {
        let mut changed = false;
        for spec in &specs {
            let Some(alias) = spec.alias.as_deref() else {
                continue;
            };
            match resolve_alias_path(
                &spec.raw_path,
                module_path,
                &aliases,
                &external_aliases,
                &alias_names,
            ) {
                AliasResolution::Product(path) => {
                    if aliases.get(alias) != Some(&path) || external_aliases.remove(alias) {
                        aliases.insert(alias.to_owned(), path);
                        changed = true;
                    }
                }
                AliasResolution::External => {
                    if aliases.remove(alias).is_some() || external_aliases.insert(alias.to_owned())
                    {
                        changed = true;
                    }
                }
                AliasResolution::Unresolved => {}
            }
        }
        if !changed {
            break;
        }
    }
    for spec in &specs {
        let unresolved = matches!(
            resolve_alias_path(
                &spec.raw_path,
                module_path,
                &aliases,
                &external_aliases,
                &alias_names,
            ),
            AliasResolution::Unresolved
        );
        let starts_with_alias = spec
            .raw_path
            .first()
            .is_some_and(|first| alias_names.contains(first));
        if unresolved && starts_with_alias {
            errors.push(format!(
                "unresolved or cyclic product import alias: {}",
                spec.raw_path.join("::")
            ));
        }
    }
    aliases
}

fn collect_use_specs(tree: &UseTree, prefix: &mut Vec<String>, specs: &mut Vec<AliasSpec>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(ident_name(&path.ident));
            collect_use_specs(&path.tree, prefix, specs);
            prefix.pop();
        }
        UseTree::Name(name) => {
            let mut raw = prefix.clone();
            raw.push(ident_name(&name.ident));
            let alias = ident_name(&name.ident);
            specs.push(AliasSpec {
                alias: Some(alias),
                raw_path: raw,
            });
        }
        UseTree::Rename(rename) => {
            let mut raw = prefix.clone();
            raw.push(ident_name(&rename.ident));
            specs.push(AliasSpec {
                alias: Some(ident_name(&rename.rename)),
                raw_path: raw,
            });
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                collect_use_specs(tree, prefix, specs);
            }
        }
        UseTree::Glob(_) => specs.push(AliasSpec {
            alias: None,
            raw_path: prefix.clone(),
        }),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AliasResolution {
    Product(Vec<String>),
    External,
    Unresolved,
}

fn resolve_alias_path(
    raw: &[String],
    module_path: &[String],
    aliases: &BTreeMap<String, Vec<String>>,
    external_aliases: &BTreeSet<String>,
    declared_aliases: &BTreeSet<String>,
) -> AliasResolution {
    let Some(first) = raw.first() else {
        return AliasResolution::External;
    };
    let mut resolved = match first.as_str() {
        "crate" => AliasResolution::Product(Vec::new()),
        "self" => AliasResolution::Product(module_path.to_vec()),
        "super" => {
            let mut parent = module_path.to_vec();
            parent.pop();
            AliasResolution::Product(parent)
        }
        first if aliases.contains_key(first) => AliasResolution::Product(aliases[first].clone()),
        first if external_aliases.contains(first) => AliasResolution::External,
        first if declared_aliases.contains(first) => AliasResolution::Unresolved,
        _ => AliasResolution::External,
    };
    for segment in raw.iter().skip(1) {
        let AliasResolution::Product(path) = &mut resolved else {
            break;
        };
        match segment.as_str() {
            "self" => {}
            "super" => {
                path.pop();
            }
            _ => path.push(segment.to_owned()),
        }
    }
    resolved
}

fn resolve_raw_use_path(
    raw: &[String],
    module_path: &[String],
    aliases: &BTreeMap<String, Vec<String>>,
) -> Option<Vec<String>> {
    let first = raw.first()?;
    let mut resolved = match first.as_str() {
        "crate" => Vec::new(),
        "self" => module_path.to_vec(),
        "super" => {
            let mut parent = module_path.to_vec();
            parent.pop();
            parent
        }
        first => aliases.get(first).cloned()?,
    };
    for segment in raw.iter().skip(1) {
        match segment.as_str() {
            "self" => {}
            "super" => {
                resolved.pop();
            }
            _ => resolved.push(segment.to_owned()),
        }
    }
    Some(resolved)
}

fn resolve_code_path(
    path: &[String],
    module_path: &[String],
    aliases: &BTreeMap<String, Vec<String>>,
) -> Option<Vec<String>> {
    let first = path.first()?;
    let mut resolved = match first.as_str() {
        "crate" => Vec::new(),
        "self" => module_path.to_vec(),
        "super" => {
            let mut parent = module_path.to_vec();
            parent.pop();
            parent
        }
        first => aliases.get(first).cloned()?,
    };
    for segment in path.iter().skip(1) {
        match segment.as_str() {
            "self" => {}
            "super" => {
                resolved.pop();
            }
            _ => resolved.push(segment.to_owned()),
        }
    }
    Some(resolved)
}

fn validate_edges(
    edges: &BTreeSet<(String, String)>,
    occurrences: &[(String, String)],
    errors: &mut Vec<String>,
) {
    let allowed: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::from([
        ("configuration", BTreeSet::new()),
        ("managed_content", BTreeSet::from(["configuration"])),
        ("source", BTreeSet::from(["configuration"])),
        ("repository", BTreeSet::from(["configuration", "source"])),
        (
            "lifecycle",
            BTreeSet::from([
                "configuration",
                "managed_content",
                "repository",
                "source",
                "platform",
            ]),
        ),
        ("platform", BTreeSet::new()),
        ("main", CONTEXTS.iter().copied().collect()),
    ]);
    for (owner, dependency) in occurrences {
        if !allowed
            .get(owner.as_str())
            .is_some_and(|dependencies| dependencies.contains(dependency.as_str()))
        {
            errors.push(format!(
                "{owner} imports inward or across a forbidden edge to {dependency}"
            ));
        }
    }
    let mut adjacency: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (owner, dependency) in edges {
        adjacency.entry(owner).or_default().insert(dependency);
    }
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for node in adjacency.keys().copied() {
        if graph_has_cycle(node, &adjacency, &mut visiting, &mut visited) {
            errors.push(format!(
                "private product module graph contains a cycle at {node}"
            ));
            break;
        }
    }
}

fn graph_has_cycle<'a>(
    node: &'a str,
    adjacency: &BTreeMap<&'a str, BTreeSet<&'a str>>,
    visiting: &mut BTreeSet<&'a str>,
    visited: &mut BTreeSet<&'a str>,
) -> bool {
    if visiting.contains(node) {
        return true;
    }
    if !visited.insert(node) {
        return false;
    }
    visiting.insert(node);
    if let Some(dependencies) = adjacency.get(node) {
        for dependency in dependencies {
            if graph_has_cycle(dependency, adjacency, visiting, visited) {
                return true;
            }
        }
    }
    visiting.remove(node);
    false
}

fn collect_rust_sources(
    directory: &Path,
    src: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
    errors: &mut Vec<String>,
) {
    if depth > MAX_SOURCE_DEPTH {
        errors.push(format!(
            "product source traversal exceeded depth limit at {}",
            directory.display()
        ));
        return;
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!(
                "cannot enumerate product source directory {}: {error}",
                directory.display()
            ));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!("cannot inspect product source entry: {error}"));
                continue;
            }
        };
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                errors.push(format!(
                    "product source entry must not be a symlink: {}",
                    path.display()
                ));
                continue;
            }
            Ok(metadata) => metadata,
            Err(error) => {
                errors.push(format!(
                    "cannot inspect product source entry {}: {error}",
                    path.display()
                ));
                continue;
            }
        };
        if metadata.is_dir() {
            collect_rust_sources(&path, src, depth + 1, files, errors);
        } else if metadata.is_file() && path.extension().is_some_and(|extension| extension == "rs")
        {
            if files.len() >= MAX_SOURCE_FILES {
                errors.push("product source traversal exceeded file limit".into());
                return;
            }
            let _ = src;
            files.push(path);
        }
    }
}

fn validate_cargo_authority(
    root: &Path,
    manifest: &ManifestInfo,
    source_files: &[PathBuf],
    graph: &ModuleGraph,
    errors: &mut Vec<String>,
) {
    let _cargo_guard = CARGO_GATE
        .lock()
        .expect("Cargo validator gate is not poisoned");
    let mut command = MetadataCommand::new();
    command
        .manifest_path(root.join("Cargo.toml"))
        .current_dir(root)
        .no_deps()
        .other_options(vec!["--locked".into(), "--offline".into()]);
    let metadata = match command.exec() {
        Ok(metadata) => metadata,
        Err(error) => {
            errors.push(format!("cargo metadata rejected product manifest: {error}"));
            return;
        }
    };
    let Some(root_package) = metadata.root_package() else {
        errors.push("cargo metadata did not identify a root package".into());
        return;
    };
    if root_package.name.as_ref() != "omnirepo" {
        errors.push(format!(
            "root Cargo package must be named omnirepo, got {}",
            root_package.name
        ));
    }
    let binary_targets: Vec<_> = root_package
        .targets
        .iter()
        .filter(|target| target.kind.iter().any(|kind| kind == &TargetKind::Bin))
        .collect();
    if binary_targets.len() != 1 {
        errors.push(format!(
            "Cargo must discover exactly one product binary target, got {}",
            binary_targets.len()
        ));
        if binary_targets.len() > 1 {
            errors.push("product must not declare multiple binary targets".into());
        }
        if manifest
            .document
            .as_ref()
            .and_then(|document| document.get("package"))
            .and_then(TomlItem::as_table)
            .and_then(|package| package.get("autobins"))
            .and_then(TomlItem::as_bool)
            == Some(false)
            && binary_targets.is_empty()
        {
            errors.push("autobins=false requires one explicit binary target".into());
        }
    } else {
        let binary = binary_targets[0];
        if binary.name != "omnirepo" {
            errors.push(format!(
                "binary name must be omnirepo; the one Cargo binary target was {}",
                binary.name
            ));
        }
        if !binary.required_features.is_empty() {
            errors.push("product binary must not be disabled by required-features".into());
        }
        let expected = root
            .join("src/main.rs")
            .to_string_lossy()
            .replace('\\', "/");
        if binary.src_path.as_str().replace('\\', "/") != expected {
            errors.push(format!(
                "product binary source must be src/main.rs, got {}",
                binary.src_path
            ));
        }
    }
    if root_package
        .targets
        .iter()
        .any(|target| target.kind.iter().any(|kind| kind == &TargetKind::Lib))
    {
        errors.push("product must expose no library target".into());
    }

    validate_metadata_workspace(root, &metadata, errors);
    validate_effective_path_dependencies(root, root_package, errors);
    validate_package_list(root, source_files, graph, errors);
    let _ = manifest;
}

fn validate_metadata_workspace(
    root: &Path,
    metadata: &cargo_metadata::Metadata,
    errors: &mut Vec<String>,
) {
    let expected: BTreeMap<&str, &str> = BTreeMap::from([
        ("tools/omnirepo-dev/Cargo.toml", "omnirepo-dev"),
        (
            "tools/omnirepo-test-support/Cargo.toml",
            "omnirepo-test-support",
        ),
    ]);
    let packages = metadata.workspace_packages();
    if packages.len() != expected.len() + 1 {
        errors.push(format!(
            "Cargo workspace must contain root plus exactly two private tools, got {} packages",
            packages.len()
        ));
    }
    let mut seen = BTreeSet::new();
    for package in packages {
        if package.manifest_path == metadata.workspace_root.join("Cargo.toml") {
            if package.name.as_ref() != "omnirepo" {
                errors.push("workspace root package identity must be omnirepo".into());
            }
            continue;
        }
        let Some(relative) = package
            .manifest_path
            .strip_prefix(metadata.workspace_root.as_path())
            .ok()
        else {
            errors.push(format!(
                "workspace member manifest escapes workspace root: {}",
                package.manifest_path
            ));
            continue;
        };
        let relative = relative.to_string().replace('\\', "/");
        let Some(expected_name) = expected.get(relative.as_str()) else {
            errors.push(format!("unexpected workspace member: {relative}"));
            continue;
        };
        if !seen.insert(relative.clone()) {
            errors.push(format!("duplicate workspace member: {relative}"));
        }
        if package.name.as_ref() != *expected_name {
            errors.push(format!(
                "workspace member package name mismatch for {relative}: {}",
                package.name
            ));
        }
        if package.publish != Some(Vec::new()) {
            errors.push(format!(
                "workspace member {relative} must set publish = false"
            ));
        }
        let expected_manifest = root.join(&relative).to_string_lossy().replace('\\', "/");
        if package.manifest_path.as_str().replace('\\', "/") != expected_manifest {
            errors.push(format!(
                "workspace member path identity mismatch for {relative}"
            ));
        }
    }
    for relative in expected.keys() {
        if !seen.contains(*relative) {
            errors.push(format!("missing workspace member: {relative}"));
        }
    }
}

fn validate_effective_path_dependencies(
    root: &Path,
    root_package: &cargo_metadata::Package,
    errors: &mut Vec<String>,
) {
    let root = match lexical_absolute(root) {
        Ok(root) => root,
        Err(error) => {
            errors.push(format!(
                "cannot identify root for Cargo dependency checks: {error}"
            ));
            return;
        }
    };
    for dependency in &root_package.dependencies {
        let Some(path) = dependency.path.as_ref() else {
            continue;
        };
        let candidate = PathBuf::from(path.as_str());
        let Some(candidate) =
            resolve_contained_path(&root, candidate.to_string_lossy().as_ref(), errors)
        else {
            continue;
        };
        if PRIVATE_TOOLS.iter().any(|(tool, _)| {
            lexical_normalize(&root.join(tool))
                .map(|tool| candidate == tool || candidate.starts_with(&tool))
                .unwrap_or(false)
        }) {
            errors.push(format!(
                "Cargo effective dependency reaches private workspace tool: {}",
                dependency.name
            ));
        }
    }
}

fn validate_package_list(
    root: &Path,
    source_files: &[PathBuf],
    graph: &ModuleGraph,
    errors: &mut Vec<String>,
) {
    let output = Command::new("cargo")
        .current_dir(root)
        .args([
            "package",
            "--locked",
            "--offline",
            "--allow-dirty",
            "--no-verify",
            "--list",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .output();
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            errors.push(format!("cannot run cargo package --list: {error}"));
            return;
        }
    };
    if !output.status.success() {
        errors.push(format!(
            "cargo package --list rejected product package: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
        return;
    }
    let listing = match String::from_utf8(output.stdout) {
        Ok(stdout) => stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(normalize_manifest_path)
            .collect::<BTreeSet<_>>(),
        Err(error) => {
            errors.push(format!("cargo package --list output is not UTF-8: {error}"));
            return;
        }
    };
    for source in source_files {
        let Some(relative) = source
            .strip_prefix(root)
            .ok()
            .map(|path| normalize_manifest_path(&path.to_string_lossy()))
        else {
            errors.push(format!(
                "source file escapes package root: {}",
                source.display()
            ));
            continue;
        };
        let runtime_source = graph.reachable.contains(Path::new(&relative));
        let test_only_source = graph.test_only.contains(Path::new(&relative));
        if runtime_source && !listing.contains(&relative) {
            errors.push(format!(
                "runtime Rust source is excluded from the package: {relative}"
            ));
        }
        if test_only_source && listing.contains(&relative) {
            errors.push(format!(
                "test-only Rust source must be excluded from the package: {relative}"
            ));
        }
        if !runtime_source && !test_only_source {
            errors.push(format!(
                "unreachable Rust source is not package-authorized: {relative}"
            ));
        }
    }
    for path in &listing {
        if is_forbidden_package_prefix(path)
            || PRIVATE_TOOLS
                .iter()
                .any(|(tool, _)| path == tool || path.starts_with(&format!("{tool}/")))
        {
            errors.push(format!("forbidden development content is packaged: {path}"));
        }
    }
}

fn is_forbidden_package_prefix(path: &str) -> bool {
    [
        "tests/", "canon/", "scripts/", "docs/", ".beads/", ".claude/", ".codex/", ".github/",
        "agent/", "agents/",
    ]
    .iter()
    .any(|prefix| path == prefix.trim_end_matches('/') || path.starts_with(prefix))
}

// Keep a tiny token helper for the historical grouped-self unit assertion.
// Production validation uses syn's structured UseTree instead of this lexer.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Ident(String),
    Punct(String),
}

fn lex(source: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, tokens: &mut Vec<Token>| {
        if !current.is_empty() {
            tokens.push(Token::Ident(std::mem::take(current)));
        }
    };
    let chars: Vec<char> = source.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        if character.is_ascii_alphanumeric() || character == '_' {
            current.push(character);
            index += 1;
        } else {
            flush(&mut current, &mut tokens);
            if character == ':' && chars.get(index + 1) == Some(&':') {
                tokens.push(Token::Punct("::".into()));
                index += 2;
            } else if !character.is_whitespace() {
                tokens.push(Token::Punct(character.to_string()));
                index += 1;
            } else {
                index += 1;
            }
        }
    }
    flush(&mut current, &mut tokens);
    tokens
}

fn scan_group_paths(
    tokens: &[Token],
    start: usize,
    base: &[String],
    _module_path: &[String],
    dependencies: &mut BTreeSet<String>,
) {
    let Some(Token::Punct(open)) = tokens.get(start) else {
        return;
    };
    if open != "{" {
        return;
    }
    let mut depth = 0usize;
    let mut begin = start + 1;
    for cursor in start + 1..=tokens.len() {
        let separator = cursor == tokens.len()
            || (depth == 0
                && matches!(tokens.get(cursor), Some(Token::Punct(value)) if value == ","
                    || (value == "}" && cursor > start)));
        if separator {
            let slice = &tokens[begin..cursor.min(tokens.len())];
            let mut path = base.to_vec();
            let mut index = 0;
            if matches!(slice.first(), Some(Token::Ident(value)) if value == "self") {
                dependencies.extend(path.iter().cloned());
            } else {
                while let Some(Token::Ident(value)) = slice.get(index) {
                    if value == "as" {
                        break;
                    }
                    path.push(value.clone());
                    index += 1;
                    if !matches!(slice.get(index), Some(Token::Punct(value)) if value == "::") {
                        break;
                    }
                    index += 1;
                }
                if let Some(context) = path.iter().find(|part| CONTEXTS.contains(&part.as_str())) {
                    dependencies.insert(context.to_owned());
                }
            }
            begin = cursor + 1;
        } else if matches!(tokens.get(cursor), Some(Token::Punct(value)) if value == "{") {
            depth += 1;
        } else if matches!(tokens.get(cursor), Some(Token::Punct(value)) if value == "}") {
            depth = depth.saturating_sub(1);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture() -> TempDir {
        // The system temp dir on macOS lives under /var/folders, and /var
        // is a symlink there; the validated topology must not traverse
        // symlinks, so fixtures live under the repository's target dir.
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("target");
        fs::create_dir_all(&base).expect("fixture base");
        let directory = tempfile::Builder::new()
            .prefix("product-module-")
            .tempdir_in(&base)
            .expect("create fixture directory");
        let root = directory.path();
        fs::create_dir_all(root.join("src/configuration")).unwrap();
        fs::create_dir_all(root.join("tools/omnirepo-dev")).unwrap();
        fs::create_dir_all(root.join("tools/omnirepo-test-support")).unwrap();
        fs::create_dir_all(root.join("vendor/helper/src")).unwrap();
        fs::write(
            root.join("vendor/helper/Cargo.toml"),
            "[package]\nname = 'helper'\nversion = '0.1.0'\nedition = '2024'\n",
        )
        .unwrap();
        fs::write(root.join("vendor/helper/src/lib.rs"), "").unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"omnirepo\"\nversion = \"0.1.0\"\nedition = \"2024\"\nexclude = [\"target\", \"tools/omnirepo-dev\", \"tools/omnirepo-test-support\", \"src/**/unit_tests.rs\"]\n\n[workspace]\nmembers = [\"tools/omnirepo-dev\", \"tools/omnirepo-test-support\"]\nexclude = [\"vendor/helper\"]\n\n[dependencies]\n").unwrap();
        fs::write(
            root.join("Cargo.lock"),
            "version = 4\n\n[[package]]\nname = \"omnirepo\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"omnirepo-dev\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"omnirepo-test-support\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        for member in ["tools/omnirepo-dev", "tools/omnirepo-test-support"] {
            let package_name = member
                .rsplit('/')
                .next()
                .expect("workspace member has a package name");
            fs::write(
                root.join(member).join("Cargo.toml"),
                format!("[package]\nname = '{package_name}'\npublish = false\n"),
            )
            .unwrap();
            fs::create_dir_all(root.join(member).join("src")).unwrap();
            fs::write(root.join(member).join("src/lib.rs"), "").unwrap();
        }
        let declarations = CONTEXTS
            .iter()
            .map(|context| format!("mod {context};"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            root.join("src/main.rs"),
            format!("{declarations}\nfn main() {{}}\n"),
        )
        .unwrap();
        for context in CONTEXTS {
            let directory = root.join("src").join(context);
            fs::create_dir_all(&directory).unwrap();
            let import = match *context {
                "managed_content" | "source" => "use crate::configuration;",
                "repository" => "use crate::source;",
                "lifecycle" => "use crate::platform;",
                _ => "",
            };
            fs::write(directory.join("mod.rs"), import).unwrap();
        }
        fs::write(root.join("src/configuration/leaf.rs"), "").unwrap();
        fs::write(root.join("src/configuration/unit_tests.rs"), "").unwrap();
        fs::write(
            root.join("src/configuration/mod.rs"),
            "mod leaf;\n#[cfg(test)] mod unit_tests;\n",
        )
        .unwrap();
        let _cargo_guard = CARGO_GATE
            .lock()
            .expect("Cargo validator gate is not poisoned");
        Command::new("cargo")
            .current_dir(root)
            .args(["generate-lockfile", "--offline"])
            .status()
            .expect("generate fixture lockfile")
            .success()
            .then_some(())
            .expect("fixture lockfile must be valid");
        directory
    }

    fn refresh_lock(root: &Path) {
        let _cargo_guard = CARGO_GATE
            .lock()
            .expect("Cargo validator gate is not poisoned");
        assert!(
            Command::new("cargo")
                .current_dir(root)
                .args(["generate-lockfile", "--offline"])
                .status()
                .expect("refresh fixture lockfile")
                .success()
        );
    }

    #[test]
    fn valid_fixture_is_accepted() {
        validate_live_root(fixture().path()).expect("valid target topology must pass");
    }

    #[test]
    fn flat_and_legacy_layout_is_rejected() {
        let absolute_fixture = fixture();
        let root = absolute_fixture.path();
        fs::write(
            root.join("src/config.rs"),
            "// crate::util\nlet _ = \"crate::util\";\n",
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), "pub mod config;").unwrap();
        fs::create_dir(root.join("src/util")).unwrap();
        let errors = validate_live_root(root).expect_err("flat anti-pattern must fail");
        assert!(errors.iter().any(|error| error.contains("library target")));
        assert!(
            errors
                .iter()
                .any(|error| error.contains("unowned top-level"))
        );
    }

    #[test]
    fn unreachable_and_excluded_runtime_source_is_rejected() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(root.join("src/configuration/leaf.rs"), "").unwrap();
        fs::write(
            root.join("Cargo.toml"),
            fs::read_to_string(root.join("Cargo.toml"))
                .unwrap()
                .replace("\"target\"", "\"src/configuration/leaf.rs\""),
        )
        .unwrap();
        fs::write(root.join("src/repository/orphan.rs"), "").unwrap();
        let errors = validate_live_root(root).expect_err("orphan and excluded files must fail");
        assert!(errors.iter().any(|error| error.contains("not reachable")));
        assert!(errors.iter().any(|error| error.contains("excluded")));
    }

    #[test]
    fn target_and_workspace_bypasses_are_rejected() {
        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest.push_str("\nautobins = false\n[[bin]]\nname = \"one\"\n[[bin]]\nname = \"two\"\n");
        manifest = manifest.replace("tools/omnirepo-test-support", "tools/extra");
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root).expect_err("target/workspace bypasses must fail");
        assert!(errors.iter().any(|error| error.contains("multiple binary")));
        assert!(
            errors
                .iter()
                .any(|error| error.contains("workspace members"))
        );
    }

    #[test]
    fn path_dependency_aliases_are_normalized_and_contained() {
        let variants = ["helper/../tools/omnirepo-dev", "../outside"];
        for path in variants {
            let fixture = fixture();
            let root = fixture.path();
            let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
            manifest = manifest.replacen("\n[dependencies]\n", "\n", 1);
            manifest.push_str(&format!(
                "\n[dependencies]\nhelper = {{ path = '{path}' }}\n"
            ));
            fs::write(root.join("Cargo.toml"), manifest).unwrap();
            let errors = validate_live_root(root).expect_err(path);
            assert!(
                errors
                    .iter()
                    .any(|error| { error.contains("workspace tool") || error.contains("escapes") }),
                "path alias {path} must fail closed: {errors:?}"
            );
        }

        let absolute_fixture = fixture();
        let root = absolute_fixture.path();
        let absolute_alias = root
            .join("tools/omnirepo-dev")
            .join("..")
            .join("omnirepo-dev");
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest = manifest.replacen("\n[dependencies]\n", "\n", 1);
        manifest.push_str(&format!(
            "\n[dependencies]\nhelper = {{ path = '{}' }}\n",
            absolute_alias.display()
        ));
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root).expect_err("absolute tool alias");
        assert!(
            errors.iter().any(|error| error.contains("workspace tool")),
            "absolute aliases must resolve to the private tool identity: {errors:?}"
        );

        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest = manifest.replacen("\n[dependencies]\n", "\n", 1);
        manifest.push_str("\n[dependencies]\nhelper = { path = 'vendor/helper' }\n");
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        refresh_lock(root);
        validate_live_root(root).expect("contained non-tool path dependencies remain allowed");
    }

    #[test]
    fn workspace_member_package_identity_and_publish_policy_are_verified() {
        let wrong_name_fixture = fixture();
        let root = wrong_name_fixture.path();
        fs::write(
            root.join("tools/omnirepo-dev/Cargo.toml"),
            "[package]\nname = 'wrong-package'\npublish = false\n",
        )
        .unwrap();
        let errors = validate_live_root(root).expect_err("wrong workspace package identity");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("package name") && error.contains("omnirepo-dev")),
            "member package names must match their declared member identity: {errors:?}"
        );

        let fixture = fixture();
        let root = fixture.path();
        fs::write(
            root.join("tools/omnirepo-test-support/Cargo.toml"),
            "[package]\nname = 'omnirepo-test-support'\npublish = true\n",
        )
        .unwrap();
        let errors = validate_live_root(root).expect_err("publishable workspace tool");
        assert!(
            errors.iter().any(|error| error.contains("publish = false")),
            "workspace tools must remain unpublished: {errors:?}"
        );
    }

    #[test]
    fn autobins_without_an_explicit_target_is_rejected() {
        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest = manifest.replace(
            "exclude = [\"target\", \"tools/omnirepo-dev\", \"tools/omnirepo-test-support\", \"src/**/unit_tests.rs\"]",
            "exclude = [\"target\", \"tools/omnirepo-dev\", \"tools/omnirepo-test-support\", \"src/**/unit_tests.rs\"]\nautobins = false",
        );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root).expect_err("disabled automatic bins need a target");
        assert!(errors.iter().any(|error| error.contains("autobins=false")));
    }

    #[test]
    fn exactly_one_cargo_discovered_binary_may_be_implicit_or_explicit() {
        validate_live_root(fixture().path())
            .expect("the conventional src/main.rs auto-bin is valid");

        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest = manifest.replace("\n[workspace]\n", "\nautobins = false\n\n[workspace]\n");
        manifest.push_str("\n[[bin]]\nname = 'omnirepo'\npath = 'src/main.rs'\n");
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        validate_live_root(root).expect("one explicit src/main.rs bin is also valid");
    }

    #[test]
    fn lexical_scanner_ignores_non_code_and_rejects_code_edge() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(
            root.join("src/platform/mod.rs"),
            "// crate::repository\nconst S: &str = \"crate::repository\";\nconst R: &str = r###\"crate::repository\"###;\nconst C: char = ':';\nuse crate::repository;\n",
        )
        .unwrap();
        let errors = validate_live_root(root).expect_err("code edge must fail");
        assert_eq!(
            errors
                .iter()
                .filter(|error| error.contains("forbidden edge"))
                .count(),
            1
        );
    }

    #[test]
    fn stacked_cfg_test_is_recursive_without_hiding_runtime_orphans() {
        let fixture = fixture();
        let root = fixture.path();
        fs::create_dir_all(root.join("src/configuration/unit_tests")).unwrap();
        fs::write(
            root.join("src/configuration/unit_tests.rs"),
            "#[cfg(test)]\n#[allow(dead_code)]\nmod nested;\n",
        )
        .unwrap();
        fs::write(
            root.join("src/configuration/unit_tests/nested.rs"),
            "use crate::platform;\n",
        )
        .unwrap();
        fs::write(
            root.join("src/configuration/hidden.rs"),
            "use crate::platform;\n",
        )
        .unwrap();

        let errors = validate_live_root(root).expect_err("unreachable runtime source must fail");
        assert!(errors.iter().any(|error| error.contains("hidden.rs")));
        assert!(!errors.iter().any(|error| error.contains("forbidden edge")));
    }

    #[test]
    fn non_test_cfg_is_not_mistaken_for_a_test_only_subtree() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(root.join("src/configuration/unit_tests.rs"), "").unwrap();
        fs::write(
            root.join("src/configuration/mod.rs"),
            "mod leaf;\n#[cfg(not(test))]\nmod runtime_only;\n#[cfg(test)]\nmod unit_tests;\n",
        )
        .unwrap();
        fs::write(
            root.join("src/configuration/runtime_only.rs"),
            "use crate::platform;\n",
        )
        .unwrap();
        let errors = validate_live_root(root).expect_err("non-test cfg must remain runtime");
        assert!(errors.iter().any(|error| error.contains("forbidden edge")));
    }

    #[test]
    fn grouped_alias_super_and_reexport_paths_are_dependency_edges() {
        let fixture = fixture();
        let root = fixture.path();
        fs::create_dir_all(root.join("src/platform/authority")).unwrap();
        fs::write(
            root.join("src/platform/mod.rs"),
            "use crate::{repository as repo, source};\nuse crate as root;\nuse root::managed_content;\npub use crate::repository::state;\nmod authority;\n",
        )
        .unwrap();
        fs::write(
            root.join("src/platform/authority/mod.rs"),
            "use super::super::repository;\n",
        )
        .unwrap();
        let errors =
            validate_live_root(root).expect_err("all product path spellings must be checked");
        assert!(
            errors
                .iter()
                .filter(|error| error.contains("forbidden edge"))
                .count()
                >= 4,
            "expected grouped, crate-alias, re-export, and super edges: {errors:?}"
        );
    }

    #[test]
    fn manifest_structure_ignores_comments_strings_and_non_package_keys() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'omnirepo'\nversion = '0.1.0'\nedition = '2024'\ndescription = 'text mentions [[bin]] and [lib]'\nexclude = ['tools/omnirepo-dev', 'tools/omnirepo-test-support', 'src/**/unit_tests.rs']\n\n[tool.metadata]\nautobins = false\n# [[bin]]\n\n[workspace]\nmembers = [\n  'tools/omnirepo-dev',\n  'tools/omnirepo-test-support',\n]\n",
        )
        .unwrap();
        validate_live_root(root).expect("manifest comments and strings must not change structure");
    }

    #[test]
    fn explicit_bin_requires_the_main_binary_path_and_package_name() {
        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest.push_str("\nautobins = false\n[[bin]]\nname = 'other'\npath = 'src/other.rs'\n");
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root).expect_err("an alternate bin must be rejected");
        assert!(errors.iter().any(|error| error.contains("binary name")));
        assert!(errors.iter().any(|error| error.contains("binary path")));
    }

    #[test]
    fn product_cannot_reach_private_workspace_tools_through_a_path_dependency() {
        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest = manifest.replacen("\n[dependencies]\n", "\n", 1);
        manifest.push_str("\n[dependencies]\nhelper = { path = 'tools/omnirepo-dev' }\n");
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors =
            validate_live_root(root).expect_err("product-to-tool path dependencies are forbidden");
        assert!(errors.iter().any(|error| error.contains("workspace tool")));
    }

    #[test]
    fn include_and_exclude_use_cargo_style_glob_patterns() {
        let fixture = fixture();
        let root = fixture.path();
        let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            manifest.replace(
                "exclude = [\"target\", \"tools/omnirepo-dev\", \"tools/omnirepo-test-support\", \"src/**/unit_tests.rs\"]",
                "exclude = ['src/**/leaf.rs']",
            ),
        )
        .unwrap();
        let errors =
            validate_live_root(root).expect_err("glob exclusion must reject runtime source");
        assert!(errors.iter().any(|error| error.contains("excluded")));

        let manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            manifest.replace(
                "exclude = ['src/**/leaf.rs']",
                "include = ['src/**/mod.rs']",
            ),
        )
        .unwrap();
        let errors =
            validate_live_root(root).expect_err("include glob must package every runtime file");
        assert!(errors.iter().any(|error| error.contains("excluded")));
    }

    #[cfg(unix)]
    #[test]
    fn source_traversal_does_not_follow_symlinked_files_or_directories() {
        use std::os::unix::fs::symlink;

        let fixture = fixture();
        let root = fixture.path();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("outside.rs"), "").unwrap();
        symlink(
            outside.path().join("outside.rs"),
            root.join("src/configuration/outside.rs"),
        )
        .unwrap();
        symlink(outside.path(), root.join("src/configuration/outside-dir")).unwrap();
        let errors = validate_live_root(root).expect_err("symlinked source must be rejected");
        assert!(errors.iter().any(|error| error.contains("symlink")));
    }

    #[test]
    fn inline_runtime_modules_are_rejected_instead_of_hidden() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(
            root.join("src/platform/mod.rs"),
            "mod hidden { use crate::repository; }\n",
        )
        .unwrap();
        let errors = validate_live_root(root).expect_err("runtime inline modules must fail closed");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("inline runtime module"))
        );
    }

    #[test]
    fn inline_test_modules_are_classified_as_test_only() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(
            root.join("src/configuration/mod.rs"),
            "mod leaf;\n#[cfg(test)]\nmod tests { use crate::platform; }\n",
        )
        .unwrap();
        fs::remove_file(root.join("src/configuration/unit_tests.rs")).unwrap();
        validate_live_root(root).expect("inline cfg(test) modules must not become runtime edges");
    }

    #[test]
    fn target_specific_and_workspace_inherited_tool_paths_are_rejected() {
        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest = manifest.replacen("\n[dependencies]\n", "\n", 1);
        manifest.push_str(
            "\n[workspace.dependencies]\nworkspace_helper = { path = 'tools/omnirepo-dev' }\n\n[dependencies]\nroot_helper = { workspace = true }\n\n[target.'cfg(unix)'.dev-dependencies]\ntarget_helper = { path = 'tools/omnirepo-test-support' }\n\n[target.x86_64-unknown-linux-gnu.build-dependencies.target_table_helper]\npath = 'tools/omnirepo-dev'\n",
        );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root).expect_err("all dependency tables must be checked");
        assert!(errors.iter().any(|error| error.contains("workspace tool")));
        assert!(
            errors
                .iter()
                .any(|error| error.contains("workspace-inherited"))
        );
    }

    #[test]
    fn dotted_dependency_tables_and_quoted_target_keys_are_rejected() {
        let bad_fixture = fixture();
        let root = bad_fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest.push_str(
            r#"
[dependencies."ordinary_helper"]
path = "tools/omnirepo-dev"

[dev-dependencies.'dev_helper']
path = 'tools/omnirepo-test-support'

[build-dependencies.build_helper]
path = "tools/omnirepo-dev"

[target.'cfg(unix)'.dev-dependencies."target_dev_helper"]
path = "tools/omnirepo-test-support"

[target.'cfg(unix)'.'build-dependencies'.target_build_helper]
path = 'tools/omnirepo-dev'
"#,
        );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors =
            validate_live_root(root).expect_err("every dotted dependency table must be checked");
        assert!(
            errors
                .iter()
                .filter(|error| error.contains("workspace tool"))
                .count()
                >= 5,
            "all ordinary, dev, build, target, and quoted dotted paths must fail: {errors:?}"
        );

        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest.push_str(
            r#"
[dependencies."helper"]
path = "vendor/helper"

[target.'cfg(unix)'.dev-dependencies."helper"]
path = 'vendor/helper'
"#,
        );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        refresh_lock(root);
        validate_live_root(root).expect("safe dotted registry dependencies remain allowed");
    }

    #[test]
    fn duplicate_relevant_tables_and_keys_fail_closed() {
        let variants = [
            (
                "duplicate package table",
                "\n[package]\nversion = '0.2.0'\n",
            ),
            (
                "duplicate package key",
                "\n# duplicate key is inserted into the existing package table\n",
            ),
            (
                "duplicate workspace table",
                "\n[workspace]\nmembers = ['tools/omnirepo-dev', 'tools/omnirepo-test-support']\n",
            ),
            (
                "duplicate workspace key",
                "\n# duplicate key is inserted into the existing workspace table\n",
            ),
            (
                "duplicate dependency table",
                "\n[dependencies]\nhelper = '1'\n[dependencies]\nother = '1'\n",
            ),
            (
                "duplicate target dependency table",
                "\n[target.'cfg(unix)'.dependencies]\nhelper = '1'\n[target.'cfg(unix)'.dependencies]\nother = '1'\n",
            ),
            (
                "duplicate binary table",
                "\nautobins = false\n[[bin]]\nname = 'omnirepo'\npath = 'src/main.rs'\n[[bin]]\nname = 'omnirepo'\npath = 'src/main.rs'\n",
            ),
        ];
        for (label, suffix) in variants {
            let fixture = fixture();
            let root = fixture.path();
            let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
            if label == "duplicate package key" {
                manifest = manifest.replace(
                    "version = \"0.1.0\"",
                    "version = \"0.1.0\"\nversion = \"0.2.0\"",
                );
            } else if label == "duplicate workspace key" {
                manifest = manifest.replace(
                    "members = [\"tools/omnirepo-dev\", \"tools/omnirepo-test-support\"]",
                    "members = [\"tools/omnirepo-dev\", \"tools/omnirepo-test-support\"]\nmembers = [\"tools/omnirepo-dev\"]",
                );
            } else if label == "duplicate binary table" {
                manifest = manifest.replace("\n[workspace]\n", &format!("{suffix}\n[workspace]\n"));
            } else {
                manifest.push_str(suffix);
            }
            fs::write(root.join("Cargo.toml"), manifest).unwrap();
            let errors = validate_live_root(root).expect_err(label);
            assert!(
                errors.iter().any(|error| error.contains("duplicate")),
                "{label} must fail closed: {errors:?}"
            );
        }

        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest = manifest.replacen("\n[dependencies]\n", "\n", 1);
        manifest.push_str("\n[dependencies]\nhelper = { path = 'one', path = 'two' }\n");
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root).expect_err("duplicate inline dependency key");
        assert!(
            errors.iter().any(|error| error.contains("duplicate")),
            "duplicate inline dependency keys must fail closed: {errors:?}"
        );
    }

    #[test]
    fn grouped_use_tree_self_paths_keep_their_base_context_edge() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(
            root.join("src/platform/mod.rs"),
            "use crate::repository::{self as repo};\nuse crate::repository::state::{self as state};\n",
        )
        .unwrap();
        let errors = validate_live_root(root)
            .expect_err("grouped self imports must retain the repository base context");
        assert!(
            errors.iter().any(|error| error.contains("forbidden edge")),
            "grouped self paths must not hide their base dependency: {errors:?}"
        );

        let tokens = lex("{ self as repository_alias }");
        let mut dependencies = BTreeSet::new();
        scan_group_paths(
            &tokens,
            0,
            &["repository".to_owned()],
            &["platform".to_owned()],
            &mut dependencies,
        );
        assert!(
            dependencies.contains("repository"),
            "a grouped self item must resolve against its group base"
        );
    }

    #[test]
    fn glob_use_edges_resolve_product_contexts_but_ignore_external_roots() {
        let cases = [
            ("use crate::repository::*;", true),
            ("use self::repository::*;", true),
            ("use super::repository::*;", true),
            ("use crate::repository as repo;\nuse repo::*;", true),
            ("use serde::repository::*;", false),
        ];
        for (source, forbidden) in cases {
            let fixture = fixture();
            let root = fixture.path();
            fs::write(root.join("src/platform/mod.rs"), source).unwrap();
            let result = validate_live_root(root);
            if forbidden {
                let errors = result.expect_err(source);
                assert!(
                    errors.iter().any(|error| error.contains("forbidden edge")),
                    "product glob edge must remain visible for {source:?}: {errors:?}"
                );
            } else {
                result.expect("external glob roots must not fabricate product edges");
            }
        }
    }

    #[test]
    fn expression_macro_literals_are_not_scanned_as_source_macros() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(
            root.join("src/platform/mod.rs"),
            "fn marker() { let _ = format_args!(\"include!\"); }\n",
        )
        .unwrap();
        validate_live_root(root)
            .expect("literal include! text inside an expression macro is not a source edge");
    }

    #[test]
    fn item_position_graph_macros_are_rejected_by_ast_kind() {
        for source in ["make_module!();\n", "include!(\"../../outside.rs\");\n"] {
            let fixture = fixture();
            let root = fixture.path();
            fs::write(root.join("src/platform/mod.rs"), source).unwrap();
            let errors = validate_live_root(root).expect_err(source);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("item-position graph macro")),
                "item-position macro must fail closed for {source:?}: {errors:?}"
            );
            if source.starts_with("include!") {
                assert!(
                    errors
                        .iter()
                        .any(|error| error.contains("include source macro")),
                    "the actual include! item macro must be identified: {errors:?}"
                );
            }
        }
    }

    #[test]
    fn declarative_macro_definitions_do_not_create_source_edges() {
        let fixture = fixture();
        fs::write(
            fixture.path().join("src/platform/mod.rs"),
            "macro_rules! make_module { () => {}; }\n",
        )
        .unwrap();
        validate_live_root(fixture.path())
            .expect("declarative macro definitions do not create module graph edges");
    }

    #[test]
    fn reverse_ordered_alias_chains_resolve_to_product_edges() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(
            root.join("src/platform/mod.rs"),
            "use first::repository as edge;\nuse second as first;\nuse crate::repository as second;\nuse edge::*;\n",
        )
        .unwrap();
        let errors = validate_live_root(root)
            .expect_err("reverse-ordered multi-hop aliases must expose their product edge");
        assert!(
            errors.iter().any(|error| error.contains("forbidden edge")),
            "reverse alias chain must resolve after a fixed point: {errors:?}"
        );
    }

    #[test]
    fn relevant_alias_cycles_fail_closed_without_rejecting_external_paths() {
        let cycle_fixture = fixture();
        let cycle_root = cycle_fixture.path();
        fs::write(
            cycle_root.join("src/platform/mod.rs"),
            "use a::repository as edge;\nuse b as a;\nuse a as b;\n",
        )
        .unwrap();
        let errors = validate_live_root(cycle_root)
            .expect_err("a cycle that reaches a product context must fail closed");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("unresolved or cyclic product import alias")),
            "relevant alias cycle must be diagnosed: {errors:?}"
        );

        let external_fixture = fixture();
        let external_root = external_fixture.path();
        fs::write(
            external_root.join("src/platform/mod.rs"),
            "use serde::repository as external;\nuse external::*;\n",
        )
        .unwrap();
        validate_live_root(external_root)
            .expect("an unresolved external alias must not fabricate a product edge");
    }

    #[test]
    fn runtime_and_test_only_reachability_conflict_is_detected_on_first_insertion() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(
            root.join("src/configuration/mod.rs"),
            "#[cfg(test)] mod leaf;\nmod leaf;\n#[cfg(test)] mod unit_tests;\n",
        )
        .unwrap();
        let errors = validate_live_root(root)
            .expect_err("a source reached first as test-only and then runtime must fail closed");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("both runtime and test-only")),
            "dual reachability must be diagnosed regardless of insertion order: {errors:?}"
        );
    }

    #[test]
    fn nested_runtime_module_visibility_must_remain_private() {
        for visibility in ["pub", "pub(crate)", "pub(super)"] {
            let fixture = fixture();
            let root = fixture.path();
            fs::write(
                root.join("src/configuration/mod.rs"),
                format!("{visibility} mod leaf;\n#[cfg(test)] mod unit_tests;\n"),
            )
            .unwrap();
            let errors = validate_live_root(root).expect_err(visibility);
            assert!(
                errors.iter().any(|error| error.contains("private")),
                "runtime module visibility {visibility} must fail closed: {errors:?}"
            );
        }
    }

    #[test]
    fn non_path_workspace_inheritance_remains_allowed() {
        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest = manifest.replacen("\n[dependencies]\n", "\n", 1);
        manifest.push_str(
            "\n[workspace.dependencies]\nhelper = { path = 'vendor/helper' }\n\n[dependencies]\nhelper = { workspace = true }\n",
        );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        refresh_lock(root);
        validate_live_root(root).expect("registry workspace inheritance is not a tool path");
    }

    #[test]
    fn unsupported_relevant_toml_shapes_fail_closed() {
        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest = manifest.replace(
            "exclude = [\"target\", \"tools/omnirepo-dev\", \"tools/omnirepo-test-support\", \"src/**/unit_tests.rs\"]",
            "exclude = { value = 'target' }",
        );
        manifest = manifest.replacen("\n[dependencies]\n", "\n", 1);
        manifest.push_str("\n[dependencies]\nhelper = { path = ['tools/omnirepo-dev'] }\n");
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root).expect_err("ambiguous Cargo shapes must fail closed");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("unsupported")
                    || error.contains("must be a string array"))
        );
    }

    #[test]
    fn unsupported_multiline_and_array_target_shapes_fail_closed() {
        let fixture = fixture();
        let root = fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest.push_str(
            "\ndescription = \"\"\"a multiline value\n\"\"\"\n[[example]]\nname = 'hidden'\n",
        );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors =
            validate_live_root(root).expect_err("unsupported manifest forms must fail closed");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("multiline") || error.contains("target table"))
        );
    }

    #[test]
    fn thin_main_rejects_top_level_logic_types_and_inline_modules() {
        let fixture = fixture();
        let root = fixture.path();
        let main = fs::read_to_string(root.join("src/main.rs")).unwrap();
        fs::write(
            root.join("src/main.rs"),
            main.replace("fn main()", "const EXTRA: usize = 1;\nfn main()")
                .replace("mod configuration;", "mod configuration { }"),
        )
        .unwrap();
        let errors =
            validate_live_root(root).expect_err("main must remain an exact composition root");
        assert!(errors.iter().any(|error| error.contains("exact private")));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_manifest_and_source_root_fail_before_content_reads() {
        use std::os::unix::fs::symlink;

        let fixture = fixture();
        let root = fixture.path();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("Cargo.toml"), "this is not Cargo").unwrap();
        fs::rename(root.join("Cargo.toml"), root.join("Cargo.real")).unwrap();
        symlink(outside.path().join("Cargo.toml"), root.join("Cargo.toml")).unwrap();
        let errors = validate_live_root(root).expect_err("symlinked manifest must fail closed");
        assert!(errors.iter().any(|error| error.contains("symlink")));

        fs::remove_file(root.join("Cargo.toml")).unwrap();
        fs::rename(root.join("Cargo.real"), root.join("Cargo.toml")).unwrap();
        let src_real = root.join("src.real");
        fs::rename(root.join("src"), &src_real).unwrap();
        symlink(&src_real, root.join("src")).unwrap();
        let errors = validate_live_root(root).expect_err("symlinked source root must fail closed");
        assert!(errors.iter().any(|error| error.contains("symlink")));
    }

    #[test]
    fn grouped_crate_root_alias_chains_are_dependency_edges() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(
            root.join("src/platform/mod.rs"),
            "use crate::{self as root};\nuse root::{self as alias};\nuse alias::repository;\n",
        )
        .unwrap();
        let errors = validate_live_root(root)
            .expect_err("grouped crate-root aliases and alias chains must remain visible");
        assert!(
            errors.iter().any(|error| error.contains("forbidden edge")),
            "grouped crate-root aliases must not bypass dependency validation: {errors:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_dependency_and_workspace_parent_paths_fail_closed() {
        use std::os::unix::fs::symlink;

        let dependency_fixture = fixture();
        let root = dependency_fixture.path();
        fs::remove_dir_all(root.join("vendor/helper")).unwrap();
        symlink(root.join("tools/omnirepo-dev"), root.join("vendor/helper")).unwrap();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest = manifest.replacen("\n[dependencies]\n", "\n", 1);
        manifest.push_str("\n[dependencies]\nhelper = { path = 'vendor/helper' }\n");
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root)
            .expect_err("symlinked dependency paths must not bypass private-tool identity");
        assert!(
            errors.iter().any(|error| error.contains("symlink")),
            "symlinked dependency path must fail closed: {errors:?}"
        );

        let workspace_fixture = fixture();
        let root = workspace_fixture.path();
        fs::rename(root.join("tools"), root.join("tools-real")).unwrap();
        symlink(root.join("tools-real"), root.join("tools")).unwrap();
        let errors = validate_live_root(root)
            .expect_err("symlinked workspace-member parents must fail before manifest reads");
        assert!(
            errors.iter().any(|error| error.contains("symlink")),
            "symlinked workspace parent must fail closed: {errors:?}"
        );
    }

    #[test]
    fn unicode_escapes_decode_before_path_and_duplicate_key_checks() {
        let path_fixture = fixture();
        let root = path_fixture.path();
        let mut manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        manifest = manifest.replacen("\n[dependencies]\n", "\n", 1);
        manifest.push_str(
            r#"
[dependencies]
helper = { path = "tools/\u006fmnirepo-dev" }
"#,
        );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root)
            .expect_err("Unicode-escaped private-tool paths must resolve to the tool identity");
        assert!(
            errors.iter().any(|error| error.contains("workspace tool")),
            "Unicode-escaped private-tool path must fail closed: {errors:?}"
        );

        let duplicate_fixture = fixture();
        let root = duplicate_fixture.path();
        let manifest = fs::read_to_string(root.join("Cargo.toml"))
            .unwrap()
            .replace(
                "name = \"omnirepo\"",
                "name = \"omnirepo\"\n\"\\u006eame\" = \"other\"",
            );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root)
            .expect_err("Unicode-escaped duplicate keys must share canonical identity");
        assert!(
            errors.iter().any(|error| error.contains("duplicate")),
            "Unicode-escaped duplicate key must fail closed: {errors:?}"
        );
    }

    #[test]
    fn path_attribute_and_include_escape_are_rejected() {
        let path_fixture = fixture();
        let root = path_fixture.path();
        fs::write(root.join("src/platform/leaf.rs"), "").unwrap();
        fs::write(
            root.join("src/platform/mod.rs"),
            "#[path = \"../../outside.rs\"]\nmod leaf;\n",
        )
        .unwrap();
        fs::write(root.join("outside.rs"), "").unwrap();
        let errors = validate_live_root(root)
            .expect_err("path attributes that escape the source tree must fail closed");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("path") || error.contains("source")),
            "escaping #[path] must be reported: {errors:?}"
        );

        let include_fixture = fixture();
        let root = include_fixture.path();
        fs::write(
            root.join("src/platform/mod.rs"),
            "include!(\"../../outside.rs\");\n",
        )
        .unwrap();
        fs::write(root.join("outside.rs"), "").unwrap();
        let errors = validate_live_root(root)
            .expect_err("include! paths that escape the source tree must fail closed");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("include") || error.contains("source")),
            "escaping include! must be reported: {errors:?}"
        );
    }

    #[test]
    fn reachable_module_declaration_cycles_fail_closed() {
        let fixture = fixture();
        let root = fixture.path();
        fs::write(
            root.join("src/configuration/mod.rs"),
            "mod leaf;\n#[cfg(test)] mod unit_tests;\n",
        )
        .unwrap();
        fs::write(root.join("src/configuration/leaf.rs"), "mod leaf;\n").unwrap();
        let errors = validate_live_root(root)
            .expect_err("reachable module-declaration cycles must not be silently revisited");
        assert!(
            errors.iter().any(|error| error.contains("cycle")),
            "module graph cycle must be reported: {errors:?}"
        );
    }

    #[test]
    fn cargo_directory_globs_apply_to_descendant_source_files() {
        let exclude_fixture = fixture();
        let root = exclude_fixture.path();
        let manifest = fs::read_to_string(root.join("Cargo.toml"))
            .unwrap()
            .replace(
                "exclude = [\"target\", \"tools/omnirepo-dev\", \"tools/omnirepo-test-support\", \"src/**/unit_tests.rs\"]",
                "exclude = ['*/configuration']",
            );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root)
            .expect_err("Cargo directory excludes must cover descendant source files");
        assert!(
            errors.iter().any(|error| error.contains("excluded")),
            "directory exclude must reject the whole context tree: {errors:?}"
        );

        let include_fixture = fixture();
        let root = include_fixture.path();
        let manifest = fs::read_to_string(root.join("Cargo.toml"))
            .unwrap()
            .replace(
                "exclude = [\"target\", \"tools/omnirepo-dev\", \"tools/omnirepo-test-support\", \"src/**/unit_tests.rs\"]",
                "include = ['*/configuration']",
            );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        let errors = validate_live_root(root)
            .expect_err("Cargo directory includes must cover only descendant source files");
        assert!(
            errors.iter().any(|error| error.contains("excluded")),
            "directory include must reject files outside the included tree: {errors:?}"
        );
    }

    #[test]
    fn private_tool_sources_must_be_excluded_from_product_package() {
        let fixture = fixture();
        let root = fixture.path();
        fs::create_dir_all(root.join("tools/omnirepo-dev/src/nested")).unwrap();
        fs::write(root.join("tools/omnirepo-dev/src/lib.rs"), "").unwrap();
        fs::write(root.join("tools/omnirepo-dev/src/nested/leaf.rs"), "").unwrap();
        let manifest = fs::read_to_string(root.join("Cargo.toml"))
            .unwrap()
            .replace(
                "exclude = [\"target\", \"tools/omnirepo-dev\", \"tools/omnirepo-test-support\", \"src/**/unit_tests.rs\"]",
                "include = [\"src/**/*\", \"tools/omnirepo-dev/src/**/*\"]",
            );
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        // Cargo excludes nested workspace packages from the root package by design.
        // Remove the member manifests for this isolated package-list probe so the
        // recursive include is exercised as a real Cargo package boundary, then
        // feed the listing through the same validator used by validate_live_root.
        let member_manifests = [
            (
                root.join("tools/omnirepo-dev/Cargo.toml"),
                fs::read_to_string(root.join("tools/omnirepo-dev/Cargo.toml")).unwrap(),
            ),
            (
                root.join("tools/omnirepo-test-support/Cargo.toml"),
                fs::read_to_string(root.join("tools/omnirepo-test-support/Cargo.toml")).unwrap(),
            ),
        ];
        let probe_manifest = fs::read_to_string(root.join("Cargo.toml")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            probe_manifest.replace(
                "members = [\"tools/omnirepo-dev\", \"tools/omnirepo-test-support\"]",
                "members = []",
            ),
        )
        .unwrap();
        for (path, _) in &member_manifests {
            fs::remove_file(path).unwrap();
        }
        let mut package_errors = Vec::new();
        validate_package_list(
            root,
            &[root.join("tools/omnirepo-dev/src/lib.rs")],
            &ModuleGraph::default(),
            &mut package_errors,
        );
        for (path, contents) in member_manifests {
            fs::write(path, contents).unwrap();
        }
        fs::write(root.join("Cargo.toml"), probe_manifest).unwrap();
        assert!(
            package_errors
                .iter()
                .any(|error| error.contains("forbidden development content is packaged")),
            "validator must reject Cargo-listed private-tool source: {package_errors:?}"
        );
    }

    #[test]
    fn nested_catch_all_module_names_are_rejected() {
        for name in ["common", "util", "utils", "prelude"] {
            let fixture = fixture();
            let root = fixture.path();
            fs::write(
                root.join("src/configuration/mod.rs"),
                format!("mod {name};\nmod leaf;\n#[cfg(test)] mod unit_tests;\n"),
            )
            .unwrap();
            fs::write(
                root.join("src/configuration").join(format!("{name}.rs")),
                "",
            )
            .unwrap();
            let errors = validate_live_root(root).expect_err(name);
            assert!(
                errors.iter().any(|error| error.contains("catch-all")),
                "nested {name} module must be rejected: {errors:?}"
            );
        }
    }

    #[test]
    fn live_repository_root_matches_the_complete_architecture_contract() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        validate_live_root(&root)
            .expect("live repository root must satisfy the complete architecture contract");
    }
}
