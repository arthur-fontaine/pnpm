//! Seed an empty `node_modules` from another git worktree.
//!
//! A fresh worktree of a repository starts without `node_modules`, so its
//! first install writes the whole tree — on a large workspace, hundreds of
//! thousands of files that a sibling worktree of the same repository
//! already holds. Cloning that tree copy-on-write costs a fraction of
//! writing it file by file, and the install that follows only reconciles
//! the difference: the clone carries `.modules.yaml` and the current
//! lockfile, so it reads as the previous install's state.
//!
//! Copy-on-write is what makes this safe to do behind the user's back:
//! the clone shares storage until one side writes, and neither worktree
//! can see the other's later edits.

use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

/// What [`seed_modules_from_worktree`] cloned.
pub struct SeededModules {
    pub donor: PathBuf,
    pub elapsed: std::time::Duration,
    pub clone_elapsed: std::time::Duration,
}

/// Clone a sibling worktree's `node_modules` into `modules_dir`.
///
/// `None` when there is nothing to seed from, when `modules_dir` already
/// exists, or when the filesystem has no directory-level clone.
pub fn seed_modules_from_worktree(
    workspace_root: &Path,
    modules_dir: &Path,
) -> Option<SeededModules> {
    if modules_dir.exists() {
        return None;
    }
    let donor = pick_donor(workspace_root, modules_dir)?;
    let donor_modules = donor.path.join("node_modules");
    let started = std::time::Instant::now();
    let clone_started = std::time::Instant::now();
    if let Err(error) = clone_dir(&donor_modules, modules_dir) {
        tracing::debug!(
            target: "pacquet::install",
            ?error,
            donor = %donor.path.display(),
            "could not clone a sibling worktree's node_modules",
        );
        return None;
    }
    let clone_elapsed = clone_started.elapsed();
    rewrite_workspace_state(modules_dir, &donor.path, workspace_root);
    Some(SeededModules { clone_elapsed, donor: donor.path, elapsed: started.elapsed() })
}

struct Donor {
    path: PathBuf,
    installed_at: std::time::SystemTime,
}

/// The worktree whose tree this install can adopt wholesale, and among
/// several the most recently installed one. Every candidate that is
/// turned down logs why at debug level.
fn pick_donor(workspace_root: &Path, modules_dir: &Path) -> Option<Donor> {
    let Ok(our_lockfile) = fs::read(workspace_root.join("pnpm-lock.yaml")) else {
        tracing::debug!(
            target: "pacquet::install",
            "no pnpm-lock.yaml here, so no worktree can be adopted",
        );
        return None;
    };
    let mut donors: Vec<Donor> = repository_worktrees(workspace_root)
        .into_iter()
        .filter(|path| path != workspace_root)
        .filter_map(|path| {
            let rejected = |reason: &'static str| -> Option<Donor> {
                tracing::debug!(
                    target: "pacquet::install",
                    candidate = %path.display(),
                    reason,
                    "worktree cannot seed this node_modules",
                );
                None
            };
            // `.modules.yaml` is what makes the tree readable as a previous
            // install; a bare `node_modules` would leave the install
            // guessing what is in it.
            let state = path.join("node_modules/.modules.yaml");
            let Some(installed_at) =
                fs::metadata(&state).ok().and_then(|state| state.modified().ok())
            else {
                return rejected("no node_modules/.modules.yaml");
            };
            // Never clone a directory into itself or into one of its own
            // parents: `modules_dir` may be a symlink into the donor.
            if modules_dir.starts_with(&path) {
                return rejected("our node_modules lives inside it");
            }
            if fs::read(path.join("pnpm-lock.yaml")).ok().as_deref() != Some(&our_lockfile) {
                return rejected("its pnpm-lock.yaml differs from ours");
            }
            // The donor's *wanted* lockfile matching ours is not enough:
            // its tree may lag its own lockfile, and then the install
            // would clone a stale tree only to rebuild all of it, which
            // costs more than installing from the store. What has to
            // match is the tree the donor actually materialized.
            if !donor_tree_is_current(&path) {
                return rejected("its tree does not hold what its lockfile asks for");
            }
            if !install_inputs_match(&path, workspace_root) {
                return rejected("its workspace or project manifests differ from ours");
            }
            Some(Donor { path, installed_at })
        })
        .collect();
    donors.sort_by_key(|donor| std::cmp::Reverse(donor.installed_at));
    donors.into_iter().next()
}

/// Whether the two worktrees describe the same install: the same
/// workspace manifest and the same project manifests, on top of the
/// identical lockfile the caller already checked.
///
/// The lockfile carries each patch's hash, so identical lockfiles mean
/// the patches the donor installed are the patches this worktree asks
/// for. A patch file edited without regenerating the lockfile is outside
/// what this can see — the same blind spot the mtime comparison it
/// stands in for has.
fn install_inputs_match(donor_root: &Path, workspace_root: &Path) -> bool {
    let same_file = |relative: &Path| {
        fs::read(donor_root.join(relative)).ok() == fs::read(workspace_root.join(relative)).ok()
    };
    if !same_file(Path::new("pnpm-workspace.yaml")) {
        return false;
    }
    let state_path = donor_root.join("node_modules/.pnpm-workspace-state-v1.json");
    let Ok(text) = fs::read_to_string(&state_path) else { return false };
    let Ok(state) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
    let Some(projects) = state.get("projects").and_then(serde_json::Value::as_object) else {
        return false;
    };
    projects.keys().all(|project| {
        Path::new(project)
            .strip_prefix(donor_root)
            .is_ok_and(|relative| same_file(&relative.join("package.json")))
    })
}

/// Whether the donor's `node_modules` holds what its lockfile asks for.
///
/// The current lockfile records what the donor's last install
/// materialized; when its snapshot set is the lockfile's, cloning the
/// tree hands the install a state it can accept wholesale. A tree that
/// lags — a lockfile pulled since the last install, an interrupted run —
/// would be cloned and then rebuilt in full.
fn donor_tree_is_current(donor_root: &Path) -> bool {
    let snapshots = |lockfile: Option<pnpm_lockfile::Lockfile>| {
        lockfile.and_then(|lockfile| lockfile.snapshots).map(|snapshots| {
            snapshots.keys().map(ToString::to_string).collect::<std::collections::BTreeSet<_>>()
        })
    };
    let current = pnpm_lockfile::Lockfile::load_current_from_virtual_store_dir(
        &donor_root.join("node_modules/.pnpm"),
    );
    let wanted = pnpm_lockfile::Lockfile::load_wanted_from_dir(donor_root);
    match (current, wanted) {
        (Ok(current), Ok(wanted)) => {
            let (current, wanted) = (snapshots(current), snapshots(wanted));
            current.is_some() && current == wanted
        }
        _ => false,
    }
}

/// Every worktree of the repository `workspace_root` belongs to, as git
/// reports them. An empty list when git is absent or this is not a
/// repository, which turns the seeding off rather than failing an install.
fn repository_worktrees(workspace_root: &Path) -> Vec<PathBuf> {
    let Ok(output) = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(workspace_root)
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .collect()
}

/// Point the cloned workspace state at this worktree. Its project keys are
/// the donor's absolute paths, which would otherwise read as a workspace
/// whose projects all moved — the state is dropped when it cannot be
/// rewritten, since a stale one only costs the install its fast path.
fn rewrite_workspace_state(modules_dir: &Path, donor_root: &Path, workspace_root: &Path) {
    let state_path = modules_dir.join(".pnpm-workspace-state-v1.json");
    let rewritten = fs::read_to_string(&state_path).ok().and_then(|text| {
        let mut state: serde_json::Value = serde_json::from_str(&text).ok()?;
        move_paths(&mut state, donor_root, workspace_root);
        // The donor validated these very bytes. A worktree checkout gives
        // every file a fresh mtime, so without carrying the validation
        // forward the freshness check reads unchanged manifests and
        // patches as edits and reinstalls the tree it was just handed.
        if let Some(timestamp) = state.get_mut("lastValidatedTimestamp")
            && let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        {
            *timestamp = serde_json::json!(now.as_millis() as i64);
        }
        serde_json::to_string(&state).ok()
    });
    match rewritten {
        Some(text) => {
            let _ = fs::write(&state_path, text);
        }
        None => {
            let _ = fs::remove_file(&state_path);
        }
    }
}

/// Repoint every path under `donor_root` — project keys, and the setting
/// values that carry absolute paths — at `workspace_root`. Matching on
/// the path prefix rather than the raw substring keeps a value that
/// merely starts with the same characters intact.
fn move_paths(value: &mut serde_json::Value, donor_root: &Path, workspace_root: &Path) {
    // A temporary directory reaches the state file through its symlink
    // (`/var/...`) or its real path (`/private/var/...`) depending on who
    // wrote the entry, so both spellings of the donor have to match.
    let donor_real = fs::canonicalize(donor_root).unwrap_or_else(|_| donor_root.to_path_buf());
    let moved = |text: &str| -> Option<String> {
        let path = Path::new(text);
        let relative =
            path.strip_prefix(donor_root).or_else(|_| path.strip_prefix(&donor_real)).ok()?;
        // `join("")` on the donor root itself would append a separator,
        // and the workspace state is keyed by exact strings.
        let moved = if relative.as_os_str().is_empty() {
            workspace_root.to_path_buf()
        } else {
            workspace_root.join(relative)
        };
        Some(moved.to_string_lossy().into_owned())
    };
    match value {
        serde_json::Value::String(text) => {
            if let Some(replacement) = moved(text) {
                *text = replacement;
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                move_paths(item, donor_root, workspace_root);
            }
        }
        serde_json::Value::Object(entries) => {
            *entries = entries
                .iter()
                .map(|(key, entry)| {
                    let mut entry = entry.clone();
                    move_paths(&mut entry, donor_root, workspace_root);
                    (moved(key).unwrap_or_else(|| key.clone()), entry)
                })
                .collect();
        }
        _ => {}
    }
}

/// Clone a directory hierarchy in one call.
///
/// APFS clones a whole tree in a single `clonefile`; every other
/// filesystem pacquet runs on reflinks per file at best, which is the
/// cost the caller is trying to avoid, so they get no seeding.
#[cfg(target_os = "macos")]
fn clone_dir(src: &Path, dst: &Path) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let src = CString::new(src.as_os_str().as_bytes())?;
    let dst = CString::new(dst.as_os_str().as_bytes())?;
    // SAFETY: both pointers come from `CString`s that outlive the call,
    // and `clonefile` only reads them.
    let status = unsafe { libc::clonefile(src.as_ptr(), dst.as_ptr(), 0) };
    if status == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

#[cfg(not(target_os = "macos"))]
fn clone_dir(_src: &Path, _dst: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "no directory-level clone on this filesystem"))
}
