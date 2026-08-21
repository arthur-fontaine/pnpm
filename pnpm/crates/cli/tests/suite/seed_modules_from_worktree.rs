//! A worktree with no `node_modules` clones one from another worktree of
//! the same repository instead of writing the whole tree from the store.

#![cfg(unix)]

use crate::_utils;
pub use _utils::*;

use assert_cmd::prelude::*;
use command_extra::CommandExtra;
use pnpm_testing_utils::bin::{AddMockedRegistry, CommandTempCwd};
use std::{fs, path::Path, process::Command};

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

/// A committed workspace with one dependency installed, plus the path a
/// sibling worktree should be created at.
fn committed_workspace(workspace: &Path) {
    fs::write(
        workspace.join("package.json"),
        serde_json::json!({
            "name": "seeded",
            "version": "1.0.0",
            "private": true,
            "dependencies": { "is-positive": "1.0.0" },
        })
        .to_string(),
    )
    .expect("write package.json");
    fs::write(workspace.join(".gitignore"), "node_modules/\n").expect("write .gitignore");
    git(workspace, &["init", "-q", "-b", "main"]);
    git(workspace, &["config", "user.email", "test@example.invalid"]);
    git(workspace, &["config", "user.name", "Test"]);
    // A contributor's global config may sign commits through an agent
    // this test can't reach.
    git(workspace, &["config", "commit.gpgsign", "false"]);
    git(workspace, &["config", "tag.gpgsign", "false"]);
    git(workspace, &["add", "-A"]);
    git(workspace, &["commit", "-qm", "init"]);
}

#[cfg(target_os = "macos")]
#[test]
fn a_sibling_worktree_clones_the_tree_instead_of_writing_it() {
    let CommandTempCwd { pacquet, root, workspace, npmrc_info, .. } =
        CommandTempCwd::init().add_mocked_registry();
    let AddMockedRegistry { mock_instance, .. } = npmrc_info;
    committed_workspace(&workspace);

    pacquet.with_arg("install").assert().success();
    // The lockfile the sibling inherits has to be the installed one.
    git(&workspace, &["add", "-A"]);
    git(&workspace, &["commit", "-qm", "lockfile"]);

    let sibling = root.path().join("sibling");
    git(&workspace, &["worktree", "add", "-q", sibling.to_str().expect("utf-8"), "HEAD"]);
    assert!(!sibling.join("node_modules").exists(), "a fresh worktree starts without a tree");

    let install = pacquet_in(&sibling).with_arg("install").assert().success();
    let output = String::from_utf8_lossy(&install.get_output().stdout).into_owned();

    assert!(output.contains("Cloned node_modules from"), "the tree is cloned: {output}");
    let manifest = sibling.join("node_modules/is-positive/package.json");
    assert!(manifest.is_file(), "the cloned tree carries the dependency: {output}");
    // The clone carries the previous install's state, which is what lets
    // the install reconcile instead of rebuilding.
    assert!(
        sibling.join("node_modules/.pnpm/lock.yaml").is_file(),
        "the clone carries the current lockfile",
    );
    assert!(
        sibling.join("node_modules/.modules.yaml").is_file(),
        "the clone carries the modules manifest",
    );
    let state = fs::read_to_string(sibling.join("node_modules/.pnpm-workspace-state-v1.json"))
        .expect("read the workspace state");
    assert!(
        !state.contains(workspace.to_str().expect("utf-8")),
        "the cloned state points at this worktree, not the donor: {state}",
    );

    drop((root, mock_instance));
}

#[cfg(target_os = "macos")]
#[test]
fn a_worktree_without_the_donor_s_lockfile_is_not_cloned() {
    let CommandTempCwd { pacquet, root, workspace, npmrc_info, .. } =
        CommandTempCwd::init().add_mocked_registry();
    let AddMockedRegistry { mock_instance, .. } = npmrc_info;
    committed_workspace(&workspace);

    pacquet.with_arg("install").assert().success();
    git(&workspace, &["add", "-A"]);
    git(&workspace, &["commit", "-qm", "lockfile"]);
    // `HEAD~1` predates the lockfile, so this worktree asks for an
    // install the donor's tree does not hold.
    let sibling = root.path().join("sibling");
    git(&workspace, &["worktree", "add", "-q", sibling.to_str().expect("utf-8"), "HEAD~1"]);

    let install = pacquet_in(&sibling).with_arg("install").assert().success();
    let output = String::from_utf8_lossy(&install.get_output().stdout).into_owned();

    assert!(!output.contains("Cloned node_modules"), "a mismatched donor is skipped: {output}");
    assert!(
        sibling.join("node_modules/is-positive/package.json").is_file(),
        "and the install writes its own tree",
    );

    drop((root, mock_instance));
}

#[cfg(target_os = "macos")]
#[test]
fn a_worktree_whose_workspace_manifest_differs_is_not_cloned() {
    let CommandTempCwd { pacquet, root, workspace, npmrc_info, .. } =
        CommandTempCwd::init().add_mocked_registry();
    let AddMockedRegistry { mock_instance, .. } = npmrc_info;
    committed_workspace(&workspace);

    pacquet.with_arg("install").assert().success();
    git(&workspace, &["add", "-A"]);
    git(&workspace, &["commit", "-qm", "lockfile"]);
    let sibling = root.path().join("sibling");
    git(&workspace, &["worktree", "add", "-q", sibling.to_str().expect("utf-8"), "HEAD"]);
    // A setting that changes what the tree has to look like, with the
    // lockfile the donor installed left untouched.
    let workspace_yaml = sibling.join("pnpm-workspace.yaml");
    let mut yaml = fs::read_to_string(&workspace_yaml).expect("read pnpm-workspace.yaml");
    yaml.push_str("nodeLinker: hoisted\n");
    fs::write(&workspace_yaml, yaml).expect("write pnpm-workspace.yaml");

    let install = pacquet_in(&sibling).with_arg("install").assert().success();
    let output = String::from_utf8_lossy(&install.get_output().stdout).into_owned();

    assert!(
        !output.contains("Cloned node_modules"),
        "a donor that would be rebuilt is skipped: {output}",
    );
    assert!(
        sibling.join("node_modules/is-positive/package.json").is_file(),
        "and the install writes its own tree",
    );

    drop((root, mock_instance));
}
