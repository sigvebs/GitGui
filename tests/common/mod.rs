//! Shared scaffolding for the integration tests: throwaway git repositories.
//!
//! Not every test file uses every helper, hence the blanket allow.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use gitgui::git::diff;
use gitgui::git::Repo;

static SEQ: AtomicUsize = AtomicUsize::new(0);

pub const BASE: &str = "one\ntwo\nthree\nfour\nfive\n";

pub struct TempRepo {
    pub dir: PathBuf,
    pub repo: Repo,
    /// Extra directories (bare remotes) to remove on drop.
    extra: Vec<PathBuf>,
}

impl TempRepo {
    pub fn new(tag: &str) -> TempRepo {
        // Keep settings writes out of the real user profile.
        let cfg = std::env::temp_dir().join(format!("gitgui-cfg-{}", std::process::id()));
        std::env::set_var("GITGUI_CONFIG_DIR", &cfg);

        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "gitgui-t-{}-{}-{}",
            std::process::id(),
            tag,
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        run(&dir, &["init", "-b", "main"]);
        run(&dir, &["config", "user.name", "Test"]);
        run(&dir, &["config", "user.email", "test@example.com"]);
        run(&dir, &["config", "commit.gpgsign", "false"]);
        run(&dir, &["config", "core.autocrlf", "false"]);

        let repo = Repo::discover(&dir).unwrap();
        TempRepo {
            dir,
            repo,
            extra: Vec::new(),
        }
    }

    /// Creates a bare repository and wires it up as a remote, so push can be
    /// exercised for real without a network or credentials.
    pub fn add_bare_remote(&mut self, name: &str) -> PathBuf {
        let bare = self.dir.with_extension(format!("{name}.git"));
        let _ = std::fs::remove_dir_all(&bare);
        std::fs::create_dir_all(&bare).unwrap();
        run(&bare, &["init", "--bare", "-b", "main"]);
        self.extra.push(bare.clone());
        self.git(&["remote", "add", name, bare.to_str().unwrap()]);
        bare
    }

    /// Resolves a ref inside a bare remote, or None if it does not exist.
    pub fn remote_sha(bare: &Path, refname: &str) -> Option<String> {
        let out = std::process::Command::new("git")
            .current_dir(bare)
            .args(["rev-parse", "--verify", "--quiet", refname])
            .output()
            .unwrap();
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    pub fn write(&self, path: &str, content: &str) {
        let full = self.dir.join(path);
        if let Some(p) = full.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(full, content).unwrap();
    }

    pub fn worktree(&self, path: &str) -> String {
        std::fs::read_to_string(self.dir.join(path)).unwrap()
    }

    /// Content of the staged (index) version.
    pub fn indexed(&self, path: &str) -> String {
        self.repo.run(&["show", &format!(":{path}")]).unwrap()
    }

    pub fn head(&self, path: &str) -> String {
        self.repo.run(&["show", &format!("HEAD:{path}")]).unwrap()
    }

    pub fn commit_all(&self, msg: &str) {
        self.repo.run(&["add", "-A"]).unwrap();
        self.repo.run(&["commit", "-m", msg]).unwrap();
    }

    pub fn git(&self, args: &[&str]) {
        run(&self.dir, args);
    }

    pub fn sha_of(&self, rev: &str) -> String {
        self.repo.run(&["rev-parse", rev]).unwrap().trim().to_string()
    }

    /// Builds a history with a root commit, an edit, a rename+edit, and a merge
    /// — the four shapes the history view has to render.
    pub fn build_history(&self) {
        self.write("a.txt", "a1\na2\na3\n");
        self.commit_all("root commit");

        self.write("a.txt", "a1\nCHANGED here\na3\n");
        self.commit_all("modify a");

        self.git(&["mv", "a.txt", "renamed.txt"]);
        self.write("renamed.txt", "a1\nCHANGED here\na3\nmore\n");
        self.commit_all("rename and edit");

        self.git(&["checkout", "-q", "-b", "side", "HEAD~1"]);
        self.write("s.txt", "side content\n");
        self.commit_all("side branch work");

        self.git(&["checkout", "-q", "main"]);
        self.git(&["merge", "-q", "--no-ff", "side", "-m", "merge side"]);
    }

    /// Leaves the repository mid-merge with one conflicted file, exactly as the
    /// terminal would: `side` and `main` both changed the same line.
    pub fn conflicted_merge(&self) {
        self.write("shared.txt", "one\ntwo\nthree\n");
        self.write("calm.txt", "untouched\n");
        self.commit_all("base");

        self.git(&["checkout", "-q", "-b", "side"]);
        self.write("shared.txt", "one\nTHEIRS\nthree\n");
        self.write("only-theirs.txt", "added on side\n");
        self.commit_all("side edit");

        self.git(&["checkout", "-q", "main"]);
        self.write("shared.txt", "one\nOURS\nthree\n");
        self.commit_all("main edit");

        // Expected to fail: that is the conflict.
        let out = std::process::Command::new("git")
            .current_dir(&self.dir)
            .args(["merge", "--no-edit", "side"])
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "the merge was supposed to conflict: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }

    pub fn unstaged_diff(&self, path: &str) -> diff::FileDiff {
        diff::parse(&self.repo.diff_unstaged(path).unwrap())
    }

    pub fn staged_diff(&self, path: &str) -> diff::FileDiff {
        diff::parse(&self.repo.diff_staged(path, None, None).unwrap())
    }
}

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
        for e in &self.extra {
            let _ = std::fs::remove_dir_all(e);
        }
    }
}

pub fn run(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Finds the (hunk, line) index of the changed line whose content matches.
pub fn locate(fd: &diff::FileDiff, needle: &str) -> (usize, usize) {
    for (hi, h) in fd.hunks.iter().enumerate() {
        for (li, l) in h.lines.iter().enumerate() {
            if l.is_change() && l.text.trim_end_matches('\r') == needle {
                return (hi, li);
            }
        }
    }
    panic!("no changed line matching {needle:?} in {fd:#?}");
}
