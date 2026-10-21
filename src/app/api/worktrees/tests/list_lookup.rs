use super::*;

fn list_entries(app: &App, root: &Path, paths: &[PathBuf]) -> Vec<WorktreeInfo> {
    let source = WorktreeSource {
        workspace_idx: None,
        source_checkout_path: root.to_path_buf(),
        source_repo_root: root.to_path_buf(),
        repo_key: "lookup-test".into(),
        repo_name: "lookup-test".into(),
    };
    let entries = paths
        .iter()
        .map(|path| crate::worktree::ExistingWorktree {
            path: path.clone(),
            branch: Some("kept-branch".into()),
            is_bare: false,
            is_detached: false,
            is_prunable: false,
        })
        .collect();
    let response = app.finish_worktree_list("lookup".into(), source, entries);
    let response: SuccessResponse = serde_json::from_str(&response).unwrap();
    let ResponseResult::WorktreeList { worktrees, .. } = response.result else {
        panic!("expected worktree list");
    };
    worktrees
}

fn explicit_workspace(path: &Path) -> Workspace {
    let mut workspace = Workspace::test_new("lookup");
    workspace.identity_cwd = path.to_path_buf();
    workspace.worktree_space = Some(crate::workspace::WorktreeSpaceMembership {
        key: "lookup-test".into(),
        label: "lookup-test".into(),
        repo_root: path.to_path_buf(),
        checkout_path: path.to_path_buf(),
        is_linked_worktree: true,
    });
    workspace
}

#[test]
fn worktree_list_lookup_preserves_first_match_and_explicit_provenance() {
    let root = unique_temp_path("list-lookup-provenance");
    let checkout = root.join("checkout");
    let navigated = root.join("navigated");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::create_dir_all(&navigated).unwrap();
    let mut first = explicit_workspace(&checkout);
    first.identity_cwd = navigated.clone();
    let second = explicit_workspace(&checkout);
    let first_id = first.id.clone();
    let mut app = test_app();
    app.state.workspaces = vec![first, second];
    let worktrees = list_entries(&app, &root, &[checkout, navigated]);
    assert_eq!(worktrees[0].open_workspace_id.as_ref(), Some(&first_id));
    assert_eq!(worktrees[1].open_workspace_id, None);
    assert_eq!(worktrees[0].branch.as_deref(), Some("kept-branch"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn worktree_list_lookup_keeps_cached_git_identity_and_current_cwd_fallback() {
    let root = unique_temp_path("list-lookup-cwd");
    let identity = root.join("identity");
    let cwd = root.join("cwd");
    let next_cwd = root.join("next");
    for path in [&identity, &cwd, &next_cwd] {
        std::fs::create_dir_all(path).unwrap();
    }
    let mut workspace = Workspace::test_new("lookup");
    workspace.identity_cwd = cwd.clone();
    workspace.cached_git_space = Some(crate::workspace::GitSpaceMetadata {
        key: "lookup-test".into(),
        checkout_key: crate::worktree::canonical_or_original(&identity)
            .display()
            .to_string(),
        repo_name: "lookup-test".into(),
        repo_root: root.clone(),
        is_linked_worktree: false,
    });
    let id = workspace.id.clone();
    let mut app = test_app();
    app.state.workspaces = vec![workspace];
    app.state.ensure_test_terminals();
    let pane = app.state.workspaces[0].tabs[0].root_pane;
    let terminal = app.state.workspaces[0].tabs[0]
        .terminal_id(pane)
        .unwrap()
        .clone();
    let paths = [identity, cwd.clone(), next_cwd.clone()];
    let worktrees = list_entries(&app, &root, &paths);
    assert_eq!(worktrees[0].open_workspace_id.as_ref(), Some(&id));
    assert_eq!(worktrees[1].open_workspace_id.as_ref(), Some(&id));
    assert_eq!(worktrees[2].open_workspace_id, None);
    app.state.terminals.get_mut(&terminal).unwrap().cwd = next_cwd;
    let worktrees = list_entries(&app, &root, &paths);
    assert_eq!(worktrees[0].open_workspace_id.as_ref(), Some(&id));
    assert_eq!(worktrees[1].open_workspace_id, None);
    assert_eq!(worktrees[2].open_workspace_id.as_ref(), Some(&id));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn worktree_list_lookup_preserves_first_match_across_cwd_and_git_keys() {
    let root = unique_temp_path("list-lookup-first-key");
    std::fs::create_dir_all(&root).unwrap();
    let mut first = Workspace::test_new("first");
    first.identity_cwd = root.clone();
    let first_id = first.id.clone();
    let mut second = Workspace::test_new("second");
    second.identity_cwd = root.join("other");
    second.cached_git_space = Some(crate::workspace::GitSpaceMetadata {
        key: "lookup-test".into(),
        checkout_key: crate::worktree::canonical_or_original(&root)
            .display()
            .to_string(),
        repo_name: "lookup-test".into(),
        repo_root: root.clone(),
        is_linked_worktree: false,
    });
    let mut app = test_app();
    app.state.workspaces = vec![first, second];
    assert_eq!(
        list_entries(&app, &root, std::slice::from_ref(&root))[0]
            .open_workspace_id
            .as_ref(),
        Some(&first_id)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn worktree_list_lookup_keeps_git_display_keys_distinct_from_path_equality() {
    use std::os::unix::ffi::OsStringExt;
    let root = unique_temp_path("list-lookup-non-utf8");
    let first_path = root.join(std::ffi::OsString::from_vec(vec![b'a', 0x80]));
    let other_path = root.join(std::ffi::OsString::from_vec(vec![b'a', 0x81]));
    std::fs::create_dir_all(&first_path).unwrap();
    std::fs::create_dir_all(&other_path).unwrap();
    let mut app = test_app();
    app.state.workspaces = vec![explicit_workspace(&first_path)];
    assert_eq!(
        list_entries(&app, &root, std::slice::from_ref(&other_path))[0].open_workspace_id,
        None
    );
    let first_id = app.state.workspaces[0].id.clone();
    app.state.workspaces[0].worktree_space = None;
    app.state.workspaces[0].cached_git_space = Some(crate::workspace::GitSpaceMetadata {
        key: "lookup-test".into(),
        checkout_key: crate::worktree::canonical_or_original(&first_path)
            .display()
            .to_string(),
        repo_name: "lookup-test".into(),
        repo_root: root.clone(),
        is_linked_worktree: false,
    });
    assert_eq!(
        list_entries(&app, &root, &[other_path])[0]
            .open_workspace_id
            .as_ref(),
        Some(&first_id)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn worktree_list_lookup_preserves_symlink_aliases_and_missing_path_equality() {
    let root = unique_temp_path("list-lookup-alias");
    let checkout = root.join("checkout");
    let alias = root.join("alias");
    let missing = root.join("missing");
    std::fs::create_dir_all(&checkout).unwrap();
    std::os::unix::fs::symlink(&checkout, &alias).unwrap();
    let first = explicit_workspace(&alias);
    let first_id = first.id.clone();
    let missing_workspace = explicit_workspace(&missing.join("."));
    let missing_id = missing_workspace.id.clone();
    let mut app = test_app();
    app.state.workspaces = vec![first, explicit_workspace(&checkout), missing_workspace];
    let worktrees = list_entries(&app, &root, &[checkout, alias, missing]);
    assert_eq!(worktrees[0].open_workspace_id.as_ref(), Some(&first_id));
    assert_eq!(worktrees[1].open_workspace_id.as_ref(), Some(&first_id));
    assert_eq!(worktrees[2].open_workspace_id.as_ref(), Some(&missing_id));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn worktree_list_lookup_canonicalization_scales_with_inputs() {
    let root = unique_temp_path("list-lookup-scale");
    std::fs::create_dir_all(&root).unwrap();
    let mut observations = Vec::new();
    for count in [16, 64, 256] {
        let mut app = test_app();
        app.state.workspaces = (0..count)
            .map(|index| {
                let path = root.join(format!("workspace-{index}"));
                std::fs::create_dir_all(&path).unwrap();
                explicit_workspace(&path)
            })
            .collect();
        let paths: Vec<_> = (0..count)
            .map(|index| {
                let path = root.join(format!("checkout-{index}"));
                std::fs::create_dir_all(&path).unwrap();
                path
            })
            .collect();
        let started = std::time::Instant::now();
        let (worktrees, calls) =
            crate::worktree::canonicalization_calls::measure(|| list_entries(&app, &root, &paths));
        println!("worktree-list scaling: entries={count} workspaces={count} canonicalizations={calls} elapsed_us={}", started.elapsed().as_micros());
        assert_eq!(worktrees.len(), count);
        assert!(worktrees
            .iter()
            .all(|entry| entry.open_workspace_id.is_none()));
        observations.push((count, calls));
    }
    std::fs::remove_dir_all(root).unwrap();
    for (count, calls) in observations {
        assert!(
            calls <= count * 3 + 1,
            "{count} worktrees/workspaces caused {calls} canonicalizations"
        );
    }
}
