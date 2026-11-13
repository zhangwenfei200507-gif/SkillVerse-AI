//! The project snapshot is a bounded projection of root metadata.
use super::*;
use crate::orchestration::security;
use std::path::{Path, PathBuf};

const TEXT_FILES: &[&str] = &[
    "package.json",
    "pyproject.toml",
    "requirements.txt",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "Cargo.toml",
    "go.mod",
    "README.md",
];
const MARKERS: &[&str] = &[
    "pnpm-lock.yaml",
    "package-lock.json",
    "yarn.lock",
    "bun.lock",
    "uv.lock",
    "poetry.lock",
    "tsconfig.json",
    "vite.config.ts",
    "vitest.config.ts",
    "jest.config.js",
    "playwright.config.ts",
    "pytest.ini",
    "gradlew",
    "src",
    "app",
    "components",
    "test",
    "tests",
    "e2e",
    "src/test",
];
const TEXT_LIMIT: u64 = 64 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct Context {
    pub name: String,
    pub project_types: Vec<String>,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub package_managers: Vec<String>,
    pub test_tools: Vec<String>,
    pub test_locations: Vec<String>,
    pub metadata_files: Vec<String>,
    pub recognized_dependencies: Vec<String>,
    pub readme_present: bool,
    pub skipped_metadata: Vec<String>,
    pub fingerprint: String,
}

pub(super) struct Selection {
    pub path: PathBuf,
    pub root_key: String,
    pub stamp: String,
}

fn stamp_entry(root: &Path, name: &str) -> Value {
    let path = root.join(name);
    if !security::ordinary(&path) {
        return json!([name, "missing_or_linked"]);
    }
    match std::fs::metadata(path) {
        Ok(m) => json!([
            name,
            m.is_file(),
            m.is_dir(),
            m.len(),
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|t| t.as_nanos())
        ]),
        Err(_) => json!([name, "unreadable"]),
    }
}

pub(super) fn select(raw: &str) -> Result<Selection> {
    if raw.len() > 2048 {
        return Err("PROJECT_ROOT_TOO_LONG".into());
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(Path::new(raw).components().next(),Some(Component::Prefix(prefix))
            if matches!(prefix.kind(),Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        {
            return Err("ROUTING_LOCAL_PROJECT_REQUIRED".into());
        }
    }
    let path = security::project(raw)?;
    let root_key = digest(path.to_string_lossy().as_bytes());
    let stamps: Vec<_> = TEXT_FILES
        .iter()
        .chain(MARKERS)
        .map(|name| stamp_entry(&path, name))
        .collect();
    Ok(Selection {
        path,
        root_key,
        stamp: digest(serde_json::to_string(&stamps).unwrap().as_bytes()),
    })
}

pub(super) fn snapshot(selected: &Selection) -> Context {
    let mut contents = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    let mut skipped = Vec::new();
    for &name in TEXT_FILES {
        let path = selected.path.join(name);
        if std::fs::symlink_metadata(&path).is_err() {
            continue;
        }
        match security::read(&path, TEXT_LIMIT) {
            Ok(bytes) => {
                hashes.insert(name, digest(&bytes));
                if name != "README.md" {
                    if let Ok(text) = String::from_utf8(bytes) {
                        contents.insert(name, text);
                    }
                }
            }
            Err(_) => skipped.push(name.to_string()),
        }
    }
    let markers: BTreeSet<&str> = MARKERS
        .iter()
        .copied()
        .filter(|name| {
            let path = selected.path.join(name);
            security::ordinary(&path) && path.exists()
        })
        .collect();
    let package = contents
        .get("package.json")
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .unwrap_or(Value::Null);
    let name = package["name"]
        .as_str()
        .filter(|name| {
            !name.is_empty()
                && name.len() <= 80
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"@/._-".contains(&b))
        })
        .unwrap_or("Selected project")
        .to_string();
    let mut types = BTreeSet::new();
    let mut languages = BTreeSet::new();
    let mut frameworks = BTreeSet::new();
    let mut managers = BTreeSet::new();
    let mut tests = BTreeSet::new();
    let mut deps = BTreeSet::new();
    let has_dep = |name: &str| {
        ["dependencies", "devDependencies", "peerDependencies"]
            .iter()
            .any(|key| package[*key].get(name).is_some())
    };
    if contents.contains_key("package.json") {
        types.insert("Node.js");
        languages.insert("JavaScript");
    }
    for (dep, label, kind) in [
        ("react", "React", "Frontend"),
        ("vue", "Vue", "Frontend"),
        ("svelte", "Svelte", "Frontend"),
        ("next", "Next.js", "Frontend"),
        ("vite", "Vite", "Frontend"),
        ("express", "Express", "Backend"),
        ("fastify", "Fastify", "Backend"),
        ("@nestjs/core", "NestJS", "Backend"),
        ("@tauri-apps/api", "Tauri", "Desktop"),
    ] {
        if has_dep(dep) {
            deps.insert(dep);
            frameworks.insert(label);
            types.insert(kind);
        }
    }
    if markers.contains("tsconfig.json") || has_dep("typescript") {
        languages.insert("TypeScript");
    }
    for (dep, config, label) in [
        ("vitest", "vitest.config.ts", "Vitest"),
        ("jest", "jest.config.js", "Jest"),
        ("@playwright/test", "playwright.config.ts", "Playwright"),
    ] {
        if has_dep(dep) || markers.contains(config) {
            tests.insert(label);
        }
    }
    for (marker, label) in [
        ("pnpm-lock.yaml", "pnpm"),
        ("package-lock.json", "npm"),
        ("yarn.lock", "Yarn"),
        ("bun.lock", "Bun"),
        ("uv.lock", "uv"),
        ("poetry.lock", "Poetry"),
        ("gradlew", "Gradle"),
    ] {
        if markers.contains(marker) {
            managers.insert(label);
        }
    }
    let python = ["pyproject.toml", "requirements.txt"]
        .iter()
        .filter_map(|key| contents.get(key))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    if contents.contains_key("pyproject.toml") || contents.contains_key("requirements.txt") {
        types.insert("Python");
        languages.insert("Python");
        if contents.contains_key("requirements.txt") {
            managers.insert("pip");
        }
        for (needle, label) in [
            ("fastapi", "FastAPI"),
            ("django", "Django"),
            ("flask", "Flask"),
        ] {
            if python.contains(needle) {
                frameworks.insert(label);
                types.insert("Backend");
                deps.insert(needle);
            }
        }
        if python.contains("pytest") || markers.contains("pytest.ini") {
            tests.insert("pytest");
        }
    }
    let jvm = ["pom.xml", "build.gradle", "build.gradle.kts"]
        .iter()
        .filter_map(|key| contents.get(key))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    if ["pom.xml", "build.gradle", "build.gradle.kts"]
        .iter()
        .any(|key| contents.contains_key(key))
    {
        types.insert("JVM");
        languages.insert("Java");
        if contents.contains_key("pom.xml") {
            managers.insert("Maven");
        }
        if contents.contains_key("build.gradle") || contents.contains_key("build.gradle.kts") {
            managers.insert("Gradle");
        }
        if jvm.contains("spring-boot") {
            frameworks.insert("Spring Boot");
            types.insert("Backend");
        }
        if jvm.contains("junit") {
            tests.insert("JUnit");
        }
    }
    if contents.contains_key("Cargo.toml") {
        types.insert("Rust");
        languages.insert("Rust");
        managers.insert("Cargo");
    }
    if contents.contains_key("go.mod") {
        types.insert("Go");
        languages.insert("Go");
        managers.insert("Go modules");
    }
    let collect = |set: BTreeSet<&str>| set.into_iter().map(str::to_string).collect();
    let fingerprint = digest(
        serde_json::to_string(&(hashes.clone(), &markers, &skipped))
            .unwrap()
            .as_bytes(),
    );
    Context {
        name,
        project_types: collect(types),
        languages: collect(languages),
        frameworks: collect(frameworks),
        package_managers: collect(managers),
        test_tools: collect(tests),
        test_locations: ["test", "tests", "e2e", "src/test"]
            .into_iter()
            .filter(|name| markers.contains(name))
            .map(str::to_string)
            .collect(),
        metadata_files: hashes.keys().copied().map(str::to_string).collect(),
        recognized_dependencies: collect(deps),
        readme_present: hashes.contains_key("README.md"),
        skipped_metadata: skipped,
        fingerprint,
    }
}
