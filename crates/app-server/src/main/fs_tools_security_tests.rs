#[cfg(unix)]
#[tokio::test]
async fn canonical_rel_path_for_write_resolves_ancestor_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("tempdir");
    let root = tokio::fs::canonicalize(dir.path())
        .await
        .expect("canonicalize root");

    let allowed = root.join("allowed");
    let denied = root.join("denied");
    tokio::fs::create_dir_all(&allowed).await.expect("mkdir allowed");
    tokio::fs::create_dir_all(&denied).await.expect("mkdir denied");

    let link_dir = allowed.join("link");
    symlink(&denied, &link_dir).expect("symlink");

    let denied_file = denied.join("file.txt");
    tokio::fs::write(&denied_file, b"hi").await.expect("write");

    let requested = link_dir.join("file.txt");
    let rel = canonical_rel_path_for_write(&root, &requested)
        .await
        .expect("canonical rel");
    assert_eq!(rel, std::path::PathBuf::from("denied/file.txt"));
}

#[cfg(unix)]
#[tokio::test]
async fn rel_path_is_secret_cannot_be_bypassed_via_symlink() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("tempdir");
    let root = tokio::fs::canonicalize(dir.path())
        .await
        .expect("canonicalize root");

    let env = root.join(".env");
    tokio::fs::write(&env, b"SECRET=1\n").await.expect("write .env");

    let link = root.join("link");
    symlink(&env, &link).expect("symlink");

    let resolved = pm_core::resolve_file(
        &root,
        std::path::Path::new("link"),
        pm_core::PathAccess::Read,
        false,
    )
        .await
        .expect("resolve");
    let rel = pm_core::modes::relative_path_under_root(&root, &resolved).expect("relative path");
    assert!(rel_path_is_secret(&rel), "expected .env to be treated as secret");
}
