// Copyright 2025 The Axvisor Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Dependency-aware test scope analysis.
//!
//! Determines which test targets (QEMU configurations, development boards) need
//! to run based on the files changed in a git commit or pull request.
//!
//! The analysis works in three phases:
//! 1. **File detection**: `git diff` identifies changed files
//! 2. **Dependency propagation**: `cargo metadata` builds the workspace dependency
//!    graph, then a reverse BFS finds all transitively affected crates
//! 3. **Target mapping**: Changed files and affected crates are mapped to concrete
//!    test targets using path-based and crate-based rules

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::process::Command;

use anyhow::{Context, Result};
use cargo_metadata::MetadataCommand;
use serde::Serialize;

/// Boolean flags indicating which test targets should run.
#[derive(Debug, Default, Serialize)]
pub struct TestScope {
    pub skip_all: bool,
    pub qemu_aarch64: bool,
    pub qemu_x86_64: bool,
    pub board_phytiumpi: bool,
    pub board_rk3568: bool,
    pub changed_crates: Vec<String>,
    pub affected_crates: Vec<String>,
}

/// Internal identifiers for concrete test targets.
#[derive(Copy, Clone)]
enum TargetId {
    QemuAarch64,
    QemuX86_64,
    BoardPhytiumpi,
    BoardRk3568,
}

/// High-level architecture categories of test targets.
#[derive(Copy, Clone, Eq, PartialEq)]
enum Arch {
    AArch64,
    X86_64,
}

/// High-level kind of test target.
#[derive(Copy, Clone, Eq, PartialEq)]
enum TargetKind {
    Qemu,
    Board,
}

/// Declarative description of each concrete test target.
struct TargetMeta {
    id: TargetId,
    arch: Arch,
    kind: TargetKind,
}

const TARGETS: &[TargetMeta] = &[
    TargetMeta {
        id: TargetId::QemuAarch64,
        arch: Arch::AArch64,
        kind: TargetKind::Qemu,
    },
    TargetMeta {
        id: TargetId::QemuX86_64,
        arch: Arch::X86_64,
        kind: TargetKind::Qemu,
    },
    TargetMeta {
        id: TargetId::BoardPhytiumpi,
        arch: Arch::AArch64,
        kind: TargetKind::Board,
    },
    TargetMeta {
        id: TargetId::BoardRk3568,
        arch: Arch::AArch64,
        kind: TargetKind::Board,
    },
];

impl TestScope {
    fn all() -> Self {
        Self {
            qemu_aarch64: true,
            qemu_x86_64: true,
            board_phytiumpi: true,
            board_rk3568: true,
            ..Default::default()
        }
    }

    fn enable(&mut self, id: TargetId) {
        match id {
            TargetId::QemuAarch64 => self.qemu_aarch64 = true,
            TargetId::QemuX86_64 => self.qemu_x86_64 = true,
            TargetId::BoardPhytiumpi => self.board_phytiumpi = true,
            TargetId::BoardRk3568 => self.board_rk3568 = true,
        }
    }

    fn enable_qemu_aarch64(&mut self) {
        self.enable(TargetId::QemuAarch64);
    }

    fn enable_qemu_x86_64(&mut self) {
        self.enable(TargetId::QemuX86_64);
    }

    fn enable_board_phytiumpi(&mut self) {
        self.enable(TargetId::BoardPhytiumpi);
    }

    fn enable_board_rk3568(&mut self) {
        self.enable(TargetId::BoardRk3568);
    }

    fn enable_where<F>(&mut self, mut pred: F)
    where
        F: FnMut(&TargetMeta) -> bool,
    {
        for meta in TARGETS.iter().filter(|m| pred(m)) {
            self.enable(meta.id);
        }
    }

    fn enable_all_qemu(&mut self) {
        self.enable_where(|m| matches!(m.kind, TargetKind::Qemu));
    }

    fn enable_all_boards(&mut self) {
        self.enable_where(|m| matches!(m.kind, TargetKind::Board));
    }

    fn enable_all_aarch64(&mut self) {
        self.enable_where(|m| m.arch == Arch::AArch64);
    }

    fn any_enabled(&self) -> bool {
        self.qemu_aarch64 || self.qemu_x86_64 || self.board_phytiumpi || self.board_rk3568
    }
}

type CrateMap = HashMap<String, String>;
type ReverseDeps = HashMap<String, HashSet<String>>;

/// Entry point: analyze changes against `base_ref` and print the result.
pub fn run(base_ref: &str) -> Result<()> {
    let scope = analyze(base_ref)?;

    // Write to $GITHUB_OUTPUT when running inside GitHub Actions.
    if let Ok(path) = std::env::var("GITHUB_OUTPUT") {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .with_context(|| format!("Failed to open GITHUB_OUTPUT at {path}"))?;
        writeln!(file, "skip_all={}", scope.skip_all)?;
        writeln!(file, "qemu_aarch64={}", scope.qemu_aarch64)?;
        writeln!(file, "qemu_x86_64={}", scope.qemu_x86_64)?;
        writeln!(file, "board_phytiumpi={}", scope.board_phytiumpi)?;
        writeln!(file, "board_rk3568={}", scope.board_rk3568)?;
    }

    println!("{}", serde_json::to_string_pretty(&scope)?);
    Ok(())
}

fn analyze(base_ref: &str) -> Result<TestScope> {
    let changed_files = get_changed_files(base_ref)?;

    eprintln!("[affected] changed files ({}):", changed_files.len());
    for f in &changed_files {
        eprintln!("  {f}");
    }

    if changed_files.is_empty() {
        eprintln!("[affected] no changes detected → skip all tests");
        return Ok(TestScope { skip_all: true, ..Default::default() });
    }

    let has_code_changes = changed_files.iter().any(|f| !is_non_code_file(f));
    if !has_code_changes {
        eprintln!("[affected] only non-code files changed → skip all tests");
        return Ok(TestScope { skip_all: true, ..Default::default() });
    }

    // Phase 1 & 2: build dependency graph and propagate changes.
    let (crate_map, reverse_deps) = build_workspace_graph()?;
    let changed_crates = map_files_to_crates(&changed_files, &crate_map);
    let affected_crates = find_all_affected(&changed_crates, &reverse_deps);

    eprintln!("[affected] directly changed crates: {:?}", changed_crates);
    eprintln!("[affected] all affected crates:     {:?}", affected_crates);

    // Phase 3: map to test targets.
    let mut scope = determine_targets(&changed_files, &changed_crates, &affected_crates);
    scope.changed_crates = sorted_vec(&changed_crates);
    scope.affected_crates = sorted_vec(&affected_crates);

    eprintln!("[affected] test scope: qemu_aarch64={} qemu_x86_64={} board_phytiumpi={} board_rk3568={}",
        scope.qemu_aarch64, scope.qemu_x86_64, scope.board_phytiumpi, scope.board_rk3568);

    Ok(scope)
}

// ---------------------------------------------------------------------------
// Phase 1: detect changed files
// ---------------------------------------------------------------------------

fn get_changed_files(base_ref: &str) -> Result<Vec<String>> {
    let try_diff = |args: &[&str]| -> Option<Vec<String>> {
        let output = Command::new("git").args(args).output().ok()?;
        if !output.status.success() {
            return None;
        }
        Some(
            String::from_utf8(output.stdout)
                .ok()?
                .lines()
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect(),
        )
    };

    // Try the requested base ref first, fall back to HEAD~1.
    if let Some(files) = try_diff(&["diff", "--name-only", base_ref]) {
        return Ok(files);
    }
    eprintln!("[affected] base ref '{base_ref}' not reachable, falling back to HEAD~1");

    try_diff(&["diff", "--name-only", "HEAD~1"])
        .context("git diff failed for both the requested base ref and HEAD~1")
}

fn is_non_code_file(path: &str) -> bool {
    const SKIP_DIRS: &[&str] = &["doc/"];
    const SKIP_EXTS: &[&str] = &[".md", ".txt", ".png", ".jpg", ".jpeg", ".svg", ".gif"];
    const SKIP_FILES: &[&str] = &["LICENSE", ".gitignore", ".gitattributes"];

    SKIP_DIRS.iter().any(|d| path.starts_with(d))
        || SKIP_EXTS.iter().any(|e| path.ends_with(e))
        || SKIP_FILES.iter().any(|f| path == *f)
}

// ---------------------------------------------------------------------------
// Phase 2: workspace dependency graph & propagation
// ---------------------------------------------------------------------------

fn build_workspace_graph() -> Result<(CrateMap, ReverseDeps)> {
    let metadata = MetadataCommand::new()
        .exec()
        .context("cargo metadata failed")?;

    let ws_root = metadata.workspace_root.as_str();
    let ws_ids: HashSet<_> = metadata.workspace_members.iter().collect();

    let mut crate_map = CrateMap::new();
    let mut id_to_name = HashMap::new();

    for pkg in &metadata.packages {
        if ws_ids.contains(&pkg.id) {
            let dir = pkg
                .manifest_path
                .parent()
                .unwrap()
                .strip_prefix(ws_root)
                .unwrap_or(pkg.manifest_path.parent().unwrap())
                .to_string();
            // Ensure the directory path ends with '/' for prefix matching.
            let dir = if dir.is_empty() { String::new() } else { format!("{dir}/") };
            crate_map.insert(pkg.name.to_string(), dir);
            id_to_name.insert(pkg.id.clone(), pkg.name.to_string());
        }
    }

    let mut reverse_deps = ReverseDeps::new();
    if let Some(resolve) = &metadata.resolve {
        for node in &resolve.nodes {
            let Some(node_name) = id_to_name.get(&node.id) else { continue };
            for dep in &node.deps {
                if let Some(dep_name) = id_to_name.get(&dep.pkg) {
                    reverse_deps
                        .entry(dep_name.to_string())
                        .or_default()
                        .insert(node_name.to_string());
                }
            }
        }
    }

    eprintln!("[affected] workspace crates: {:?}", crate_map.keys().collect::<Vec<_>>());
    eprintln!("[affected] reverse deps:");
    for (k, v) in &reverse_deps {
        eprintln!("  {k} ← {:?}", v);
    }

    Ok((crate_map, reverse_deps))
}

fn map_files_to_crates(files: &[String], crate_map: &CrateMap) -> HashSet<String> {
    let mut result = HashSet::new();
    for file in files {
        // Pick the longest matching prefix to handle nested crate directories.
        let mut best: Option<&str> = None;
        for (name, dir) in crate_map {
            if !dir.is_empty() && file.starts_with(dir.as_str()) {
                if best.is_none() || dir.len() > crate_map[best.unwrap()].len() {
                    best = Some(name.as_str());
                }
            }
        }
        if let Some(name) = best {
            result.insert(name.to_string());
        }
    }
    result
}

fn find_all_affected(changed: &HashSet<String>, reverse_deps: &ReverseDeps) -> HashSet<String> {
    let mut affected = changed.clone();
    let mut queue: VecDeque<_> = changed.iter().cloned().collect();

    while let Some(current) = queue.pop_front() {
        if let Some(dependents) = reverse_deps.get(&current) {
            for dep in dependents {
                if affected.insert(dep.clone()) {
                    queue.push_back(dep.clone());
                }
            }
        }
    }
    affected
}

// ---------------------------------------------------------------------------
// Phase 3: map affected crates + changed files → test targets
// ---------------------------------------------------------------------------

fn determine_targets(
    changed_files: &[String],
    changed_crates: &HashSet<String>,
    affected_crates: &HashSet<String>,
) -> TestScope {
    let mut scope = TestScope::default();

    // ── Rule 1: root build config changes → run everything ──
    if changed_files.iter().any(|f| {
        matches!(f.as_str(), "Cargo.toml" | "Cargo.lock" | "rust-toolchain.toml")
    }) {
        return TestScope::all();
    }

    // ── Rule 2: build-tool (xtask) changes → run everything ──
    if changed_crates.contains("xtask") {
        return TestScope::all();
    }

    // ── Rule 3: core module *directly* changed → run everything ──
    //   axruntime and axconfig are foundational. Only trigger all tests when their
    //   source code is directly modified, not when they are transitively affected
    //   by a platform-specific crate (e.g. axplat-x86-qemu-q35 is a target-cfg dep
    //   of axruntime, but a change there should only require x86 testing).
    if ["axruntime", "axconfig"]
        .iter()
        .any(|c| changed_crates.contains(*c))
    {
        return TestScope::all();
    }

    // ── Rule 4: kernel common code (non-arch-specific) → run everything ──
    if changed_files.iter().any(|f| {
        f.starts_with("kernel/") && !f.starts_with("kernel/src/hal/arch/")
    }) {
        return TestScope::all();
    }

    // ── Rule 5: architecture-specific kernel code ──
    for file in changed_files {
        if file.starts_with("kernel/src/hal/arch/aarch64/") {
            scope.enable_all_aarch64();
        }
        if file.starts_with("kernel/src/hal/arch/x86_64/") {
            scope.enable_qemu_x86_64();
        }
    }

    // ── Rule 6: platform crate ──
    if affected_crates.contains("axplat-x86-qemu-q35") {
        scope.enable_qemu_x86_64();
    }

    // ── Rule 7: filesystem module → targets with `fs` feature ──
    if affected_crates.contains("axfs") {
        scope.enable_qemu_aarch64(); // linux guest uses rootfs
        scope.enable_all_boards();
    }

    // ── Rule 8: driver module → board-specific analysis ──
    if affected_crates.contains("driver") {
        let phytium = changed_files.iter().any(|f| f.contains("phytium"));
        let rockchip = changed_files
            .iter()
            .any(|f| f.contains("rockchip") || f.contains("rk3568"));
        let common_driver = changed_files.iter().any(|f| {
            f.starts_with("modules/driver/")
                && !f.contains("phytium")
                && !f.contains("rockchip")
                && !f.contains("rk3568")
        });

        if common_driver {
            scope.enable_all_boards();
        }
        if phytium {
            scope.enable_board_phytiumpi();
        }
        if rockchip {
            scope.enable_board_rk3568();
        }
    }

    // ── Rule 9: CI workflow / config file changes ──
    for file in changed_files {
        if file.starts_with(".github/workflows/") {
            if file.contains("qemu") {
                scope.enable_all_qemu();
            }
            if file.contains("board") || file.contains("uboot") {
                scope.enable_all_boards();
            }
        }
    }

    // ── Rule 10: board / VM config file changes ──
    for file in changed_files {
        if file.starts_with("configs/board/") {
            if file.contains("qemu-aarch64") {
                scope.enable_qemu_aarch64();
            }
            if file.contains("qemu-x86_64") {
                scope.enable_qemu_x86_64();
            }
            if file.contains("phytiumpi") {
                scope.enable_board_phytiumpi();
            }
            if file.contains("roc-rk3568") {
                scope.enable_board_rk3568();
            }
        }
        if file.starts_with("configs/vms/") {
            if file.contains("aarch64") {
                scope.enable_qemu_aarch64();
                if file.contains("e2000") {
                    scope.enable_board_phytiumpi();
                }
                if file.contains("rk3568") {
                    scope.enable_board_rk3568();
                }
            }
            if file.contains("x86_64") {
                scope.enable_qemu_x86_64();
            }
        }
    }

    // If nothing was enabled after all rules, treat as "skip all".
    if !scope.any_enabled() {
        scope.skip_all = true;
    }

    scope
}

fn sorted_vec(set: &HashSet<String>) -> Vec<String> {
    let mut v: Vec<_> = set.iter().cloned().collect();
    v.sort();
    v
}
